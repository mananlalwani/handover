use std::fs::Permissions;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use handover_core::{NotificationCommand, StateEvent};
use handover_ipc::{
    ErrorCode, IpcError, Method, PROTOCOL_VERSION, Request, ServerMessage, ServerPayload,
    read_json_line, runtime_directory, socket_path, write_json_line,
};
use thiserror::Error;
use tokio::io::{AsyncWrite, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;
use tracing::{debug, warn};

use crate::state::{CommandValidationError, StateSnapshot, StateStore};

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

    loop {
        if let Some(receiver) = subscription.as_mut() {
            tokio::select! {
                request = read_json_line::<_, Request>(&mut reader) => {
                    if !handle_request_result(request, &mut writer, &state, &events, &mut subscription).await? {
                        return Ok(());
                    }
                }
                event = receiver.recv() => {
                    match event {
                        Ok(event) => write_json_line(&mut writer, &message_from_event(event)).await?,
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            debug!(skipped, "IPC client lagged; sending current snapshot");
                            write_json_line(
                                &mut writer,
                                &ServerMessage::new(snapshot_payload(&state)),
                            ).await?;
                        }
                        Err(broadcast::error::RecvError::Closed) => return Ok(()),
                    }
                }
            }
        } else {
            let request = read_json_line::<_, Request>(&mut reader).await;
            if !handle_request_result(request, &mut writer, &state, &events, &mut subscription)
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
        Method::Subscribe => {
            *subscription = Some(events.subscribe());
            let snapshot = snapshot(state);
            ServerPayload::Subscribed {
                devices: snapshot.devices,
                notifications: snapshot.notifications,
            }
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
    };
    write_json_line(writer, &ServerMessage::new(response)).await?;
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

fn snapshot(state: &Arc<RwLock<StateStore>>) -> StateSnapshot {
    state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .snapshot()
}

fn snapshot_payload(state: &Arc<RwLock<StateStore>>) -> ServerPayload {
    let snapshot = snapshot(state);
    ServerPayload::Snapshot {
        devices: snapshot.devices,
        notifications: snapshot.notifications,
    }
}

fn message_from_event(event: StateEvent) -> ServerMessage {
    match event {
        StateEvent::Device(event) => ServerMessage::from_device_event(event),
        StateEvent::Notification(event) => ServerMessage::from_notification_event(event),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use handover_core::{
        BatteryState, Capability, Device, DeviceEvent, DeviceId, Notification, NotificationEvent,
        NotificationId,
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

        assert_eq!(
            snapshot_payload(&state),
            ServerPayload::Snapshot {
                devices: vec![device("Phone", 72)],
                notifications: vec![current],
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
}
