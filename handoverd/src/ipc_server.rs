use std::fs::Permissions;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use handover_core::{Capability, DeviceId, MediaCommand, NotificationCommand, StateEvent};
use handover_ipc::{
    ErrorCode, IpcError, Method, PROTOCOL_VERSION, Request, ServerMessage, ServerPayload,
    read_json_line, runtime_directory, socket_path, write_json_line,
};
use thiserror::Error;
use tokio::io::{AsyncWrite, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;
use tracing::{debug, warn};
use url::Url;

use crate::state::{CommandValidationError, MediaValidationError, StateSnapshot, StateStore};

pub(crate) const EVENT_CAPACITY: usize = 64;

pub(crate) struct IpcServer {
    listener: UnixListener,
    socket_path: PathBuf,
    state: Arc<RwLock<StateStore>>,
    events: broadcast::Sender<StateEvent>,
}

impl IpcServer {
    pub(crate) async fn bind(
        state: Arc<RwLock<StateStore>>,
        events: broadcast::Sender<StateEvent>,
    ) -> Result<Self, ServerError> {
        let directory = runtime_directory()?;
        tokio::fs::create_dir_all(&directory).await?;
        tokio::fs::set_permissions(&directory, Permissions::from_mode(0o700)).await?;
        Self::bind_at(socket_path()?, state, events).await
    }

    async fn bind_at(
        path: PathBuf,
        state: Arc<RwLock<StateStore>>,
        events: broadcast::Sender<StateEvent>,
    ) -> Result<Self, ServerError> {
        remove_stale_socket(&path).await?;
        let listener = UnixListener::bind(&path)?;
        tokio::fs::set_permissions(&path, Permissions::from_mode(0o600)).await?;
        Ok(Self {
            listener,
            socket_path: path,
            state,
            events,
        })
    }

    pub(crate) async fn run(&self) -> Result<(), ServerError> {
        loop {
            let (stream, _address) = self.listener.accept().await?;
            let state = Arc::clone(&self.state);
            let events = self.events.clone();
            tokio::spawn(async move {
                if let Err(error) = handle_client(stream, state, events).await {
                    debug!(%error, "IPC client disconnected");
                }
            });
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.socket_path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            warn!(%error, path = %self.socket_path.display(), "failed to remove IPC socket");
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum ServerError {
    #[error(transparent)]
    Ipc(#[from] IpcError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("another handoverd is already listening at {0}")]
    AlreadyRunning(PathBuf),
    #[error("refusing to remove non-socket path {0}")]
    UnsafeSocketPath(PathBuf),
}

async fn remove_stale_socket(path: &Path) -> Result<(), ServerError> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_socket() {
        return Err(ServerError::UnsafeSocketPath(path.to_owned()));
    }
    if UnixStream::connect(path).await.is_ok() {
        return Err(ServerError::AlreadyRunning(path.to_owned()));
    }
    tokio::fs::remove_file(path).await?;
    Ok(())
}

async fn handle_client(
    stream: UnixStream,
    state: Arc<RwLock<StateStore>>,
    events: broadcast::Sender<StateEvent>,
) -> Result<(), IpcError> {
    let (read_half, mut writer) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut subscription: Option<broadcast::Receiver<StateEvent>> = None;
    let mut include_share_events = false;
    let mut include_media_events = false;

    loop {
        if let Some(receiver) = subscription.as_mut() {
            tokio::select! {
                request = read_json_line::<_, Request>(&mut reader) => {
                    if !handle_request_result(request, &mut writer, &state, &events, &mut subscription, &mut include_share_events, &mut include_media_events).await? {
                        return Ok(());
                    }
                }
                event = receiver.recv() => {
                    match event {
                        Ok(event) if matches!(event, StateEvent::ShareReceived(_)) && !include_share_events => {}
                        Ok(event) if matches!(event, StateEvent::Media(_)) && !include_media_events => {}
                        Ok(event) => write_json_line(&mut writer, &message_from_event(event)).await?,
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            debug!(skipped, "IPC client lagged; sending current snapshot");
                            write_json_line(
                                &mut writer,
                                &ServerMessage::new(snapshot_payload(&state, include_media_events)),
                            ).await?;
                        }
                        Err(broadcast::error::RecvError::Closed) => return Ok(()),
                    }
                }
            }
        } else {
            let request = read_json_line::<_, Request>(&mut reader).await;
            if !handle_request_result(
                request,
                &mut writer,
                &state,
                &events,
                &mut subscription,
                &mut include_share_events,
                &mut include_media_events,
            )
            .await?
            {
                return Ok(());
            }
        }
    }
}

async fn handle_request_result<W>(
    request: Result<Option<Request>, IpcError>,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    events: &broadcast::Sender<StateEvent>,
    subscription: &mut Option<broadcast::Receiver<StateEvent>>,
    include_share_events: &mut bool,
    include_media_events: &mut bool,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let request = match request {
        Ok(Some(request)) => request,
        Ok(None) => return Ok(false),
        Err(IpcError::Json(error)) => {
            write_json_line(
                writer,
                &ServerMessage::protocol_error(ErrorCode::MalformedRequest, error.to_string()),
            )
            .await?;
            return Ok(true);
        }
        Err(error @ IpcError::LineTooLong) => {
            write_json_line(
                writer,
                &ServerMessage::protocol_error(ErrorCode::MalformedRequest, error.to_string()),
            )
            .await?;
            return Ok(false);
        }
        Err(error) => return Err(error),
    };

    if request.protocol != PROTOCOL_VERSION {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(
                ErrorCode::UnsupportedProtocol,
                format!("supported protocol is {PROTOCOL_VERSION}"),
            ),
        )
        .await?;
        return Ok(true);
    }

    let response = match request.method {
        Method::Hello => ServerPayload::Hello {
            supported_protocols: vec![PROTOCOL_VERSION],
        },
        Method::DevicesList => ServerPayload::Devices {
            devices: snapshot(state).devices,
        },
        Method::NotificationsList => ServerPayload::Notifications {
            notifications: snapshot(state).notifications,
        },
        Method::MediaList => ServerPayload::Media {
            media_sessions: snapshot(state).media_sessions,
        },
        Method::Subscribe { shares, media } => {
            *subscription = Some(events.subscribe());
            *include_share_events = shares;
            *include_media_events = media;
            let snapshot = snapshot(state);
            ServerPayload::Subscribed {
                devices: snapshot.devices,
                notifications: snapshot.notifications,
                media_sessions: if media {
                    snapshot.media_sessions
                } else {
                    Vec::new()
                },
            }
        }
        Method::MediaControl { command } => {
            return handle_media_command(command, writer, state).await;
        }
        Method::NotificationDismiss { notification_id } => {
            return handle_notification_command(
                NotificationCommand::Dismiss { notification_id },
                writer,
                state,
            )
            .await;
        }
        Method::NotificationInvokeAction {
            notification_id,
            action_id,
        } => {
            return handle_notification_command(
                NotificationCommand::InvokeAction {
                    notification_id,
                    action_id,
                },
                writer,
                state,
            )
            .await;
        }
        Method::NotificationReply {
            notification_id,
            text,
        } => {
            return handle_notification_command(
                NotificationCommand::Reply {
                    notification_id,
                    text,
                },
                writer,
                state,
            )
            .await;
        }
        Method::ShareUrl { device_id, url } => {
            return handle_share_command(device_id, url, false, writer, state).await;
        }
        Method::ShareFile {
            device_id,
            file_url,
        } => {
            return handle_share_command(device_id, file_url, true, writer, state).await;
        }
    };
    write_json_line(writer, &ServerMessage::new(response)).await?;
    Ok(true)
}

async fn handle_media_command<W>(
    command: MediaCommand,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let validation = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .validate_media_command(&command);
    if let Err(error) = validation {
        let (code, message) = match error {
            MediaValidationError::NotFound => (
                ErrorCode::MediaSessionNotFound,
                "media session is no longer available",
            ),
            MediaValidationError::DeviceUnavailable => (
                ErrorCode::DeviceDisconnected,
                "source device is unavailable",
            ),
            MediaValidationError::UnsupportedControl => (
                ErrorCode::InvalidMediaCommand,
                "media session does not support that control",
            ),
            MediaValidationError::InvalidValue => (
                ErrorCode::InvalidMediaCommand,
                "invalid media control value",
            ),
        };
        write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
        return Ok(true);
    }

    let id = command.id().clone();
    match handover_kdeconnect::KdeConnectBackend::execute_media(&command).await {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::MediaAccepted { id }),
            )
            .await?
        }
        Err(error) => {
            warn!(device_id = %id.device_id, player_id = %id.player_id, %error, "KDE Connect rejected media command");
            write_json_line(
                writer,
                &ServerMessage::protocol_error(
                    ErrorCode::BackendRejected,
                    "media backend could not accept the command",
                ),
            )
            .await?;
        }
    }
    Ok(true)
}

async fn handle_notification_command<W>(
    command: NotificationCommand,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let validation = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .validate_command(&command);
    if let Err(error) = validation {
        let (code, message) = match error {
            CommandValidationError::NotFound => (
                ErrorCode::NotificationNotFound,
                "notification is no longer available",
            ),
            CommandValidationError::NotClearable => (
                ErrorCode::InvalidNotificationCommand,
                "notification is not clearable",
            ),
            CommandValidationError::UnknownAction => (
                ErrorCode::InvalidNotificationCommand,
                "notification does not provide that action",
            ),
            CommandValidationError::ReplyUnsupported => (
                ErrorCode::InvalidNotificationCommand,
                "notification does not support replies",
            ),
            CommandValidationError::EmptyReply => (
                ErrorCode::InvalidNotificationCommand,
                "reply text must not be empty",
            ),
        };
        write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
        return Ok(true);
    }

    let notification_id = command.notification_id().clone();
    match handover_kdeconnect::KdeConnectBackend::execute(&command).await {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::CommandCompleted { notification_id }),
            )
            .await?;
        }
        Err(error) => {
            warn!(%error, %notification_id, "KDE Connect rejected notification command");
            write_json_line(
                writer,
                &ServerMessage::protocol_error(
                    ErrorCode::BackendRejected,
                    "notification backend could not complete the command",
                ),
            )
            .await?;
        }
    }
    Ok(true)
}

async fn handle_share_command<W>(
    device_id: DeviceId,
    value: String,
    is_file: bool,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let target = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_device(&device_id)
        .cloned();
    let rejected = match target {
        None => Some((ErrorCode::UnknownDevice, "device is not known")),
        Some(device) if !device.connected => {
            Some((ErrorCode::DeviceDisconnected, "device is disconnected"))
        }
        Some(device) if !device.paired => {
            Some((ErrorCode::DeviceNotPaired, "device is not paired"))
        }
        Some(device) if !device.capabilities.contains(&Capability::FileTransfer) => Some((
            ErrorCode::UnsupportedCapability,
            "device does not provide file and URL sharing",
        )),
        Some(_) => None,
    };
    if let Some((code, message)) = rejected {
        write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
        return Ok(true);
    }

    let resource = if is_file {
        validate_file_url(&value).await
    } else {
        validate_outgoing_url(&value)
    };
    let url = match resource {
        Ok(url) => url,
        Err(code) => {
            let message = match code {
                ErrorCode::ResourceNotFound => "file does not exist",
                _ => "invalid or unreadable share resource",
            };
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            return Ok(true);
        }
    };

    match handover_kdeconnect::KdeConnectBackend::share_url(&device_id, &url).await {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::ShareAccepted { device_id }),
            )
            .await?;
        }
        Err(error) => {
            let unavailable =
                matches!(error, handover_kdeconnect::CommandError::BackendUnavailable);
            warn!(%device_id, is_file, unavailable, "KDE Connect rejected share request");
            write_json_line(
                writer,
                &ServerMessage::protocol_error(
                    if unavailable {
                        ErrorCode::BackendUnavailable
                    } else {
                        ErrorCode::BackendRejected
                    },
                    if unavailable {
                        "sharing backend is unavailable"
                    } else {
                        "sharing backend could not accept the request"
                    },
                ),
            )
            .await?;
        }
    }
    Ok(true)
}

fn validate_outgoing_url(input: &str) -> Result<String, ErrorCode> {
    if input.is_empty() || input.trim() != input || input.chars().any(char::is_control) {
        return Err(ErrorCode::InvalidResource);
    }
    let url = Url::parse(input).map_err(|_| ErrorCode::InvalidResource)?;
    if matches!(url.scheme(), "file" | "javascript" | "data") {
        return Err(ErrorCode::InvalidResource);
    }
    Ok(url.into())
}

async fn validate_file_url(input: &str) -> Result<String, ErrorCode> {
    let url = Url::parse(input).map_err(|_| ErrorCode::InvalidResource)?;
    if url.scheme() != "file" || url.query().is_some() || url.fragment().is_some() {
        return Err(ErrorCode::InvalidResource);
    }
    let path = url.to_file_path().map_err(|_| ErrorCode::InvalidResource)?;
    let metadata = tokio::fs::metadata(&path).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ErrorCode::ResourceNotFound
        } else {
            ErrorCode::InvalidResource
        }
    })?;
    if !metadata.is_file() {
        return Err(ErrorCode::InvalidResource);
    }
    tokio::fs::File::open(&path)
        .await
        .map_err(|_| ErrorCode::InvalidResource)?;
    Ok(url.into())
}

fn snapshot(state: &Arc<RwLock<StateStore>>) -> StateSnapshot {
    state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .snapshot()
}

fn snapshot_payload(state: &Arc<RwLock<StateStore>>, include_media: bool) -> ServerPayload {
    let snapshot = snapshot(state);
    ServerPayload::Snapshot {
        devices: snapshot.devices,
        notifications: snapshot.notifications,
        media_sessions: if include_media {
            snapshot.media_sessions
        } else {
            Vec::new()
        },
    }
}

fn message_from_event(event: StateEvent) -> ServerMessage {
    match event {
        StateEvent::Device(event) => ServerMessage::from_device_event(event),
        StateEvent::Notification(event) => ServerMessage::from_notification_event(event),
        StateEvent::Media(event) => ServerMessage::from_media_event(event),
        StateEvent::ShareReceived(share) => {
            ServerMessage::new(ServerPayload::ShareReceived { share })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use handover_core::{
        BatteryState, Capability, Device, DeviceEvent, DeviceId, MediaCommand, MediaControl,
        MediaEvent, MediaSession, MediaSessionId, Notification, NotificationEvent, NotificationId,
        PlaybackState,
    };
    use handover_ipc::{Client, ServerPayload};
    use tempfile::TempDir;
    use tokio::io::{AsyncWriteExt, BufReader};

    use super::*;

    fn device(name: &str, percentage: u8) -> Device {
        Device {
            id: DeviceId::new(name.to_lowercase()),
            name: name.into(),
            connected: true,
            paired: true,
            battery: Some(BatteryState::new(percentage, false).expect("valid battery")),
            capabilities: BTreeSet::from([Capability::Battery]),
        }
    }

    fn notification() -> Notification {
        Notification {
            id: NotificationId::new(DeviceId::new("phone"), "1"),
            app_name: "Messages".into(),
            title: "Alice".into(),
            body: "Hello".into(),
            icon_path: None,
            clearable: true,
            actions: Vec::new(),
            reply_supported: true,
        }
    }

    fn media_session() -> MediaSession {
        MediaSession {
            id: MediaSessionId::new(DeviceId::new("phone"), "Player"),
            application: "Player".into(),
            title: Some("Test song".into()),
            artist: None,
            album: None,
            playback: PlaybackState::Paused,
            position_ms: Some(0),
            duration_ms: None,
            volume_percent: None,
            controls: BTreeSet::from([MediaControl::Play]),
        }
    }

    #[tokio::test]
    async fn media_snapshot_subscription_and_events_are_consistent() {
        let (_directory, path, state, events, task) = server_with_device().await;
        let session = media_session();
        state
            .write()
            .unwrap()
            .apply(StateEvent::Media(MediaEvent::Added(session.clone())));
        let mut client = Client::connect_to(path.clone()).await.expect("connect");
        assert_eq!(
            client.media_sessions().await.expect("list"),
            vec![session.clone()]
        );
        let client = Client::connect_to(path).await.expect("connect");
        let mut subscription = client.subscribe().await.expect("subscribe");
        assert_eq!(subscription.media_sessions, vec![session.clone()]);
        let mut updated = session;
        updated.playback = PlaybackState::Playing;
        state
            .write()
            .unwrap()
            .apply(StateEvent::Media(MediaEvent::Updated(updated.clone())));
        events
            .send(StateEvent::Media(MediaEvent::Updated(updated.clone())))
            .expect("subscriber");
        assert_eq!(
            subscription.next_message().await.expect("event").payload,
            ServerPayload::MediaUpdated {
                media_session: updated
            }
        );
        task.abort();
    }

    #[tokio::test]
    async fn media_preflight_rejects_unknown_and_unsupported_controls() {
        let (_directory, path, state, _events, task) = server_with_device().await;
        let mut client = Client::connect_to(path).await.expect("connect");
        let id = media_session().id;
        assert!(matches!(
            client
                .media_command(MediaCommand::Play { id: id.clone() })
                .await,
            Err(IpcError::Server {
                code: ErrorCode::MediaSessionNotFound,
                ..
            })
        ));
        state
            .write()
            .unwrap()
            .apply(StateEvent::Media(MediaEvent::Added(media_session())));
        assert!(matches!(
            client.media_command(MediaCommand::Pause { id }).await,
            Err(IpcError::Server {
                code: ErrorCode::InvalidMediaCommand,
                ..
            })
        ));
        task.abort();
    }

    async fn server_with_device() -> (
        TempDir,
        PathBuf,
        Arc<RwLock<StateStore>>,
        broadcast::Sender<StateEvent>,
        tokio::task::JoinHandle<()>,
    ) {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("handoverd.sock");
        let mut store = StateStore::default();
        store.apply(StateEvent::Device(DeviceEvent::Added(device("Phone", 72))));
        let state = Arc::new(RwLock::new(store));
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let server = IpcServer::bind_at(path.clone(), Arc::clone(&state), events.clone())
            .await
            .expect("server binds");
        let task = tokio::spawn(async move {
            server.run().await.expect("server runs");
        });
        (directory, path, state, events, task)
    }

    #[tokio::test]
    async fn serves_device_snapshot() {
        let (_directory, path, _state, _events, task) = server_with_device().await;
        let mut client = Client::connect_to(path).await.expect("client connects");

        let devices = client.devices().await.expect("snapshot succeeds");

        assert_eq!(devices, vec![device("Phone", 72)]);
        task.abort();
    }

    #[tokio::test]
    async fn subscription_receives_device_event() {
        let (_directory, path, state, events, task) = server_with_device().await;
        let client = Client::connect_to(path).await.expect("client connects");
        let mut subscription = client.subscribe().await.expect("subscription succeeds");
        let updated = device("Phone", 71);
        state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .apply(StateEvent::Device(DeviceEvent::Updated(updated.clone())));

        events
            .send(StateEvent::Device(DeviceEvent::Updated(updated.clone())))
            .expect("subscriber exists");
        let message = subscription.next_message().await.expect("event arrives");

        assert_eq!(
            message.payload,
            ServerPayload::DeviceUpdated { device: updated }
        );
        task.abort();
    }

    #[tokio::test]
    async fn serves_multiple_clients_after_disconnect() {
        let (_directory, path, _state, _events, task) = server_with_device().await;
        let first = Client::connect_to(path.clone())
            .await
            .expect("first client connects");
        let mut second = Client::connect_to(path)
            .await
            .expect("second client connects");
        drop(first);

        assert_eq!(
            second.devices().await.expect("second client works"),
            vec![device("Phone", 72)]
        );
        task.abort();
    }

    #[tokio::test]
    async fn notification_snapshot_and_subscription_are_consistent() {
        let (_directory, path, state, events, task) = server_with_device().await;
        let current = notification();
        state
            .write()
            .unwrap()
            .apply(StateEvent::Notification(NotificationEvent::Added(
                current.clone(),
            )));
        let mut client = Client::connect_to(path.clone()).await.expect("connect");
        assert_eq!(
            client.notifications().await.expect("list"),
            vec![current.clone()]
        );

        let client = Client::connect_to(path).await.expect("connect");
        let mut subscription = client.subscribe().await.expect("subscribe");
        assert_eq!(subscription.notifications, vec![current.clone()]);

        let mut updated = current.clone();
        updated.title = "Alice again".into();
        state
            .write()
            .unwrap()
            .apply(StateEvent::Notification(NotificationEvent::Updated(
                updated.clone(),
            )));
        events
            .send(StateEvent::Notification(NotificationEvent::Updated(
                updated.clone(),
            )))
            .expect("subscriber");
        assert_eq!(
            subscription.next_message().await.expect("event").payload,
            ServerPayload::NotificationUpdated {
                notification: updated
            }
        );
        task.abort();
    }

    #[tokio::test]
    async fn rejects_unknown_notification_and_action_without_dbus() {
        let (_directory, path, state, _events, task) = server_with_device().await;
        let mut client = Client::connect_to(path).await.expect("connect");
        let error = client
            .dismiss_notification(notification().id.clone())
            .await
            .expect_err("unknown notification");
        assert!(matches!(
            error,
            IpcError::Server {
                code: ErrorCode::NotificationNotFound,
                ..
            }
        ));

        let current = notification();
        state
            .write()
            .unwrap()
            .apply(StateEvent::Notification(NotificationEvent::Added(
                current.clone(),
            )));
        let error = client
            .invoke_notification_action(current.id, "unknown".into())
            .await
            .expect_err("unknown action");
        assert!(matches!(
            error,
            IpcError::Server {
                code: ErrorCode::InvalidNotificationCommand,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn lag_recovery_snapshot_contains_both_state_maps() {
        let (_directory, _path, state, _events, task) = server_with_device().await;
        let current = notification();
        state
            .write()
            .unwrap()
            .apply(StateEvent::Notification(NotificationEvent::Added(
                current.clone(),
            )));

        let media = media_session();
        state
            .write()
            .unwrap()
            .apply(StateEvent::Media(MediaEvent::Added(media.clone())));

        assert_eq!(
            snapshot_payload(&state, true),
            ServerPayload::Snapshot {
                devices: vec![device("Phone", 72)],
                notifications: vec![current],
                media_sessions: vec![media],
            }
        );
        task.abort();
    }

    #[tokio::test]
    async fn malformed_input_gets_controlled_error() {
        let (_directory, path, _state, _events, task) = server_with_device().await;
        let mut stream = UnixStream::connect(path)
            .await
            .expect("raw client connects");
        stream
            .write_all(b"this is not json\n")
            .await
            .expect("write succeeds");
        let (reader, _writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        let response: ServerMessage = read_json_line(&mut reader)
            .await
            .expect("read succeeds")
            .expect("response exists");

        assert!(matches!(
            response.payload,
            ServerPayload::Error {
                code: ErrorCode::MalformedRequest,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn unsupported_protocol_gets_controlled_error() {
        let (_directory, path, _state, _events, task) = server_with_device().await;
        let mut stream = UnixStream::connect(path)
            .await
            .expect("raw client connects");
        stream
            .write_all(b"{\"protocol\":2,\"method\":\"hello\"}\n")
            .await
            .expect("write succeeds");
        let (reader, _writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        let response: ServerMessage = read_json_line(&mut reader)
            .await
            .expect("read succeeds")
            .expect("response exists");

        assert!(matches!(
            response.payload,
            ServerPayload::Error {
                code: ErrorCode::UnsupportedProtocol,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn bounded_event_channel_reports_lag() {
        let (events, mut receiver) = broadcast::channel(2);
        for percentage in [70, 69, 68] {
            events
                .send(StateEvent::Device(DeviceEvent::Updated(device(
                    "Phone", percentage,
                ))))
                .expect("receiver exists");
        }

        assert!(matches!(
            receiver.recv().await,
            Err(broadcast::error::RecvError::Lagged(1))
        ));
    }

    #[test]
    fn outgoing_url_validation_accepts_real_urls_and_rejects_unsafe_values() {
        assert_eq!(
            validate_outgoing_url("https://example.com/a?q=1"),
            Ok("https://example.com/a?q=1".into())
        );
        assert!(validate_outgoing_url("mailto:someone@example.com").is_ok());
        for input in [
            "",
            "example.com",
            " https://example.com",
            "javascript:alert(1)",
            "file:///tmp/x",
            "https://example.com\n",
        ] {
            assert_eq!(
                validate_outgoing_url(input),
                Err(ErrorCode::InvalidResource)
            );
        }
    }

    #[tokio::test]
    async fn file_validation_accepts_regular_unicode_file_and_rejects_other_paths() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("handover ✓ test.txt");
        tokio::fs::write(&path, b"harmless test content")
            .await
            .expect("test file");
        let file_url = Url::from_file_path(&path).expect("file URL").to_string();
        assert_eq!(validate_file_url(&file_url).await, Ok(file_url.clone()));
        let link = directory.path().join("linked file.txt");
        std::os::unix::fs::symlink(&path, &link).expect("create test symlink");
        let link_url = Url::from_file_path(&link).expect("file URL").to_string();
        assert_eq!(validate_file_url(&link_url).await, Ok(link_url));
        let missing = Url::from_file_path(directory.path().join("missing.txt"))
            .expect("file URL")
            .to_string();
        assert_eq!(
            validate_file_url(&missing).await,
            Err(ErrorCode::ResourceNotFound)
        );
        assert_eq!(
            validate_file_url(Url::from_file_path(directory.path()).unwrap().as_ref()).await,
            Err(ErrorCode::InvalidResource)
        );
        assert_eq!(
            validate_file_url(&format!("{file_url}?unexpected=1")).await,
            Err(ErrorCode::InvalidResource)
        );
    }

    #[tokio::test]
    async fn share_preflight_rejects_unknown_device_and_unsupported_plugin() {
        let (_directory, path, _state, _events, task) = server_with_device().await;
        let mut client = Client::connect_to(path).await.expect("client connects");
        let unknown = client
            .send_url(DeviceId::new("unknown"), "https://example.com".into())
            .await
            .expect_err("unknown target");
        assert!(matches!(
            unknown,
            IpcError::Server {
                code: ErrorCode::UnknownDevice,
                ..
            }
        ));
        let unsupported = client
            .send_url(DeviceId::new("phone"), "https://example.com".into())
            .await
            .expect_err("unsupported plugin");
        assert!(matches!(
            unsupported,
            IpcError::Server {
                code: ErrorCode::UnsupportedCapability,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn share_preflight_rejects_missing_file_before_dbus() {
        let (_directory, path, state, _events, task) = server_with_device().await;
        let mut capable = device("Phone", 72);
        capable.capabilities.insert(Capability::FileTransfer);
        state
            .write()
            .unwrap()
            .apply(StateEvent::Device(DeviceEvent::Updated(capable)));
        let mut client = Client::connect_to(path).await.expect("client connects");
        let missing =
            Url::from_file_path("/tmp/handover-definitely-missing-test.txt").expect("file URL");
        let error = client
            .send_file_url(DeviceId::new("phone"), missing.into())
            .await
            .expect_err("missing file");
        assert!(matches!(
            error,
            IpcError::Server {
                code: ErrorCode::ResourceNotFound,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn share_preflight_rejects_disconnected_unpaired_and_invalid_url() {
        let (_directory, path, state, _events, task) = server_with_device().await;
        let mut client = Client::connect_to(path).await.expect("client connects");
        let mut target = device("Phone", 72);
        target.capabilities.insert(Capability::FileTransfer);
        target.connected = false;
        state
            .write()
            .unwrap()
            .apply(StateEvent::Device(DeviceEvent::Updated(target.clone())));
        assert!(matches!(
            client
                .send_url(target.id.clone(), "https://example.com".into())
                .await,
            Err(IpcError::Server {
                code: ErrorCode::DeviceDisconnected,
                ..
            })
        ));

        target.connected = true;
        target.paired = false;
        state
            .write()
            .unwrap()
            .apply(StateEvent::Device(DeviceEvent::Updated(target.clone())));
        assert!(matches!(
            client
                .send_url(target.id.clone(), "https://example.com".into())
                .await,
            Err(IpcError::Server {
                code: ErrorCode::DeviceNotPaired,
                ..
            })
        ));

        target.paired = true;
        state
            .write()
            .unwrap()
            .apply(StateEvent::Device(DeviceEvent::Updated(target.clone())));
        assert!(matches!(
            client.send_url(target.id, "not a URL".into()).await,
            Err(IpcError::Server {
                code: ErrorCode::InvalidResource,
                ..
            })
        ));
        assert!(matches!(
            client
                .send_url(DeviceId::new("phone"), "not a URL".into())
                .await,
            Err(IpcError::Server {
                code: ErrorCode::InvalidResource,
                ..
            })
        ));
        assert_eq!(
            client
                .devices()
                .await
                .expect("connection stays usable")
                .len(),
            1
        );
        task.abort();
    }

    #[tokio::test]
    async fn incoming_share_reaches_subscriber_without_entering_snapshot() {
        let (_directory, path, state, events, task) = server_with_device().await;
        let client = Client::connect_to(path).await.expect("client connects");
        let mut subscription = client.subscribe().await.expect("subscription succeeds");
        let share = handover_core::ReceivedShare {
            device_id: DeviceId::new("phone"),
            resource: handover_core::SharedResource::Url {
                url: "https://example.com".into(),
            },
        };
        let event = StateEvent::ShareReceived(share.clone());
        state.write().unwrap().apply(event.clone());
        events.send(event).expect("subscriber exists");
        assert_eq!(
            subscription
                .next_message()
                .await
                .expect("share event")
                .payload,
            ServerPayload::ShareReceived { share }
        );
        assert!(state.read().unwrap().snapshot().notifications.is_empty());
        task.abort();
    }

    #[tokio::test]
    async fn legacy_subscriber_does_not_receive_new_share_event() {
        let (_directory, path, _state, events, task) = server_with_device().await;
        let mut stream = UnixStream::connect(path).await.expect("connect");
        stream
            .write_all(b"{\"protocol\":1,\"method\":\"subscribe\"}\n")
            .await
            .expect("subscribe");
        let (reader, _writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let initial: ServerMessage = read_json_line(&mut reader)
            .await
            .expect("read")
            .expect("response");
        assert!(matches!(initial.payload, ServerPayload::Subscribed { .. }));
        events
            .send(StateEvent::ShareReceived(handover_core::ReceivedShare {
                device_id: DeviceId::new("phone"),
                resource: handover_core::SharedResource::Url {
                    url: "https://example.com".into(),
                },
            }))
            .expect("subscriber");
        events
            .send(StateEvent::Device(DeviceEvent::Updated(device(
                "Phone", 71,
            ))))
            .expect("subscriber");
        let next: ServerMessage = read_json_line(&mut reader)
            .await
            .expect("read")
            .expect("event");
        assert!(matches!(next.payload, ServerPayload::DeviceUpdated { .. }));
        task.abort();
    }

    #[tokio::test]
    async fn legacy_subscriber_does_not_receive_media_events() {
        let (_directory, path, _state, events, task) = server_with_device().await;
        let mut stream = UnixStream::connect(path).await.expect("connect");
        stream
            .write_all(b"{\"protocol\":1,\"method\":\"subscribe\"}\n")
            .await
            .expect("subscribe");
        let (reader, _writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let initial: ServerMessage = read_json_line(&mut reader)
            .await
            .expect("read")
            .expect("response");
        assert!(matches!(initial.payload, ServerPayload::Subscribed { .. }));
        events
            .send(StateEvent::Media(MediaEvent::Added(media_session())))
            .expect("subscriber");
        events
            .send(StateEvent::Device(DeviceEvent::Updated(device(
                "Phone", 70,
            ))))
            .expect("subscriber");
        let next: ServerMessage = read_json_line(&mut reader)
            .await
            .expect("read")
            .expect("event");
        assert!(matches!(next.payload, ServerPayload::DeviceUpdated { .. }));
        task.abort();
    }
}
