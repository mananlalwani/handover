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

use crate::messaging::{HistoryGap, MessagingValidationError};
use crate::messaging_backend::{HelperCallError, MessagingHub};
use crate::state::{CommandValidationError, MediaValidationError, StateSnapshot, StateStore};
use crate::{apply_backend_event, native_backend};
use handover_core::DeviceEvent;
use handover_core::{ConversationId, MessageId, MessagingAccountId, MessagingCommand};

pub(crate) const EVENT_CAPACITY: usize = 64;

#[derive(Default)]
struct SubscriptionFlags {
    shares: bool,
    media: bool,
    messages: bool,
}

pub(crate) struct IpcServer {
    listener: UnixListener,
    socket_path: PathBuf,
    state: Arc<RwLock<StateStore>>,
    events: broadcast::Sender<StateEvent>,
    messaging: Option<MessagingHub>,
}

impl IpcServer {
    pub(crate) async fn bind(
        state: Arc<RwLock<StateStore>>,
        events: broadcast::Sender<StateEvent>,
    ) -> Result<Self, ServerError> {
        let directory = runtime_directory()?;
        tokio::fs::create_dir_all(&directory).await?;
        tokio::fs::set_permissions(&directory, Permissions::from_mode(0o700)).await?;
        Self::bind_at(socket_path()?, state, events, None).await
    }

    pub(crate) fn with_messaging(mut self, hub: MessagingHub) -> Self {
        self.messaging = Some(hub);
        self
    }

    async fn bind_at(
        path: PathBuf,
        state: Arc<RwLock<StateStore>>,
        events: broadcast::Sender<StateEvent>,
        messaging: Option<MessagingHub>,
    ) -> Result<Self, ServerError> {
        remove_stale_socket(&path).await?;
        let listener = UnixListener::bind(&path)?;
        tokio::fs::set_permissions(&path, Permissions::from_mode(0o600)).await?;
        Ok(Self {
            listener,
            socket_path: path,
            state,
            events,
            messaging,
        })
    }

    pub(crate) async fn run(&self) -> Result<(), ServerError> {
        loop {
            let (stream, _address) = self.listener.accept().await?;
            let state = Arc::clone(&self.state);
            let events = self.events.clone();
            let messaging = self.messaging.clone();
            tokio::spawn(async move {
                if let Err(error) = handle_client(stream, state, events, messaging).await {
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
    messaging: Option<MessagingHub>,
) -> Result<(), IpcError> {
    let (read_half, mut writer) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut subscription: Option<broadcast::Receiver<StateEvent>> = None;
    let mut flags = SubscriptionFlags::default();

    loop {
        if let Some(receiver) = subscription.as_mut() {
            tokio::select! {
                request = read_json_line::<_, Request>(&mut reader) => {
                    if !handle_request_result(request, &mut writer, &state, &events, &messaging, &mut subscription, &mut flags).await? {
                        return Ok(());
                    }
                }
                event = receiver.recv() => {
                    match event {
                        Ok(event) if matches!(event, StateEvent::ShareReceived(_) | StateEvent::ShareResult(_)) && !flags.shares => {}
                        Ok(event) if matches!(event, StateEvent::Media(_)) && !flags.media => {}
                        Ok(event) if matches!(event, StateEvent::Messaging(_)) && !flags.messages => {}
                        Ok(event) => write_json_line(&mut writer, &message_from_event(event)).await?,
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            debug!(skipped, "IPC client lagged; sending current snapshot");
                            write_json_line(
                                &mut writer,
                                &ServerMessage::new(snapshot_payload(&state, flags.media, flags.messages)),
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
                &messaging,
                &mut subscription,
                &mut flags,
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
    messaging: &Option<MessagingHub>,
    subscription: &mut Option<broadcast::Receiver<StateEvent>>,
    flags: &mut SubscriptionFlags,
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
        Method::NativePeers => ServerPayload::NativePeers {
            peers: native_backend().map_or_else(Vec::new, |native| {
                native
                    .peers()
                    .into_iter()
                    .map(|peer| handover_ipc::NativePeer {
                        id: peer.id,
                        name: peer.name,
                        fingerprint: peer.fingerprint,
                    })
                    .collect()
            }),
        },
        Method::NativePending => ServerPayload::NativePending {
            pending: native_backend().map_or_else(Vec::new, |native| {
                native
                    .pending()
                    .into_iter()
                    .map(|peer| handover_ipc::NativePendingPeer {
                        id: peer.id,
                        name: peer.name,
                        code: peer.code,
                    })
                    .collect()
            }),
        },
        Method::NativePair { id, code } => {
            match native_backend().map(|native| native.approve(&id, &code)) {
                Some(Ok(())) => ServerPayload::NativeAccepted,
                Some(Err(_)) => ServerPayload::Error {
                    code: ErrorCode::BackendRejected,
                    message: "unknown pending peer or comparison code".into(),
                },
                None => ServerPayload::Error {
                    code: ErrorCode::BackendUnavailable,
                    message: "native backend unavailable".into(),
                },
            }
        }
        Method::NativeUnpair { id } => match native_backend().map(|native| native.unpair(&id)) {
            Some(Ok(true)) => {
                apply_backend_event(
                    state,
                    events,
                    StateEvent::Device(DeviceEvent::Removed(DeviceId::new(format!("native:{id}")))),
                );
                ServerPayload::NativeAccepted
            }
            Some(Ok(false)) => ServerPayload::Error {
                code: ErrorCode::UnknownDevice,
                message: "unknown native peer".into(),
            },
            Some(Err(_)) => ServerPayload::Error {
                code: ErrorCode::BackendRejected,
                message: "native unpair failed".into(),
            },
            None => ServerPayload::Error {
                code: ErrorCode::BackendUnavailable,
                message: "native backend unavailable".into(),
            },
        },
        Method::NativeCall {
            id,
            action,
            address,
        } => {
            let action = match action.as_str() {
                "place" => Some(handover_core::CallAction::Place),
                "answer" => Some(handover_core::CallAction::Answer),
                "decline" => Some(handover_core::CallAction::Decline),
                "hangup" => Some(handover_core::CallAction::Hangup),
                _ => None,
            };
            match action {
                Some(action) => route_call(
                    state,
                    DeviceId::new(format!("native:{id}")),
                    action,
                    address,
                ),
                None => ServerPayload::Error {
                    code: ErrorCode::BackendRejected,
                    message: "unknown call action".into(),
                },
            }
        }
        Method::CallsControl {
            device_id,
            action,
            address,
        } => route_call(state, device_id, action, address),
        Method::NotificationsList => ServerPayload::Notifications {
            notifications: snapshot(state).notifications,
        },
        Method::CallsAudio => ServerPayload::CallAudio {
            status: crate::call_audio::inspect().await,
        },
        Method::CallsList => ServerPayload::Calls {
            calls: snapshot(state).calls,
        },
        Method::MediaList => ServerPayload::Media {
            media_sessions: snapshot(state).media_sessions,
        },
        Method::Subscribe {
            shares,
            media,
            messages,
        } => {
            *subscription = Some(events.subscribe());
            flags.shares = shares;
            flags.media = media;
            flags.messages = messages;
            let snapshot = snapshot(state);
            let messaging = messaging_snapshot(state);
            ServerPayload::Subscribed {
                devices: snapshot.devices,
                calls: snapshot.calls,
                notifications: snapshot.notifications,
                media_sessions: if media {
                    snapshot.media_sessions
                } else {
                    Vec::new()
                },
                messaging_accounts: if messages {
                    messaging.accounts
                } else {
                    Vec::new()
                },
                conversations: if messages {
                    messaging.conversations
                } else {
                    Vec::new()
                },
                typing_states: if messages {
                    messaging.typing
                } else {
                    Vec::new()
                },
                read_states: if messages { messaging.read } else { Vec::new() },
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
        Method::MessagesAccounts => ServerPayload::Accounts {
            accounts: messaging_snapshot(state).accounts,
        },
        Method::MessagesConversations { account_id } => {
            return handle_conversations(account_id, writer, state).await;
        }
        Method::MessagesHistory {
            conversation_id,
            limit,
            cursor,
        } => {
            return handle_history(conversation_id, limit, cursor, writer, state, messaging).await;
        }
        Method::MessagesTypingStates => ServerPayload::TypingStates {
            states: messaging_snapshot(state).typing,
        },
        Method::MessagesReadStates => ServerPayload::ReadStates {
            states: messaging_snapshot(state).read,
        },
        Method::MessagesSend {
            conversation_id,
            text,
            reply_to,
        } => {
            return handle_messaging_send(
                conversation_id,
                text,
                reply_to,
                writer,
                state,
                messaging,
            )
            .await;
        }
        Method::MessagesSendFile {
            conversation_id,
            file_url,
            caption,
        } => {
            return handle_send_file(conversation_id, file_url, caption, writer, state, messaging)
                .await;
        }
        Method::MessagesReact { message_id, emoji } => {
            return handle_react(message_id, emoji, true, writer, state, messaging).await;
        }
        Method::MessagesUnreact { message_id, emoji } => {
            return handle_react(message_id, emoji, false, writer, state, messaging).await;
        }
        Method::MessagesRead {
            conversation_id,
            message_id,
        } => {
            return handle_mark_read(conversation_id, message_id, writer, state, messaging).await;
        }
        Method::MessagesTyping { conversation_id } => {
            return handle_typing(conversation_id, writer, state, messaging).await;
        }
        Method::MessagesDelete { message_id } => {
            return handle_delete_message(message_id, writer, state, messaging).await;
        }
        Method::MessagesOpen {
            account_id,
            addresses,
        } => {
            return handle_open_conversation(account_id, addresses, writer, state, messaging).await;
        }
        Method::MessagesLogin {
            account_id,
            bundle_b64,
        } => {
            return handle_login(account_id, bundle_b64, writer, messaging).await;
        }
        Method::MessagesLogout { account_id } => {
            return handle_logout(account_id, writer, messaging).await;
        }
        Method::MessagesSync { account_id } => {
            return handle_sync(account_id, writer, state, messaging).await;
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
    if id.device_id.as_str().starts_with("native:") {
        return handle_native_media_command(command, id, writer).await;
    }
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

/// Route a validated media command to the paired native session.
/// A successful response means the command was accepted for delivery to the
/// phone, not that Android changed playback. Authoritative state arrives as
/// a later media update.
async fn handle_native_media_command<W>(
    command: MediaCommand,
    id: handover_core::MediaSessionId,
    writer: &mut W,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let Some(native) = native_backend() else {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(
                ErrorCode::BackendUnavailable,
                "native backend unavailable",
            ),
        )
        .await?;
        return Ok(true);
    };
    let peer_id = id
        .device_id
        .as_str()
        .strip_prefix("native:")
        .unwrap_or_default();
    match native.execute_media(peer_id, &command) {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::MediaAccepted { id }),
            )
            .await?;
        }
        Err(error) => {
            use handover_native::NativeCommandError;
            let (code, message) = match error {
                NativeCommandError::Offline => (
                    ErrorCode::DeviceDisconnected,
                    "native device is disconnected",
                ),
                NativeCommandError::QueueFull => (
                    ErrorCode::BackendRejected,
                    "native backend could not accept the command",
                ),
            };
            warn!(device_id = %id.device_id, player_id = %id.player_id, "native media command not accepted");
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
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
    if notification_id.device_id.as_str().starts_with("native:") {
        return handle_native_notification_command(command, notification_id, writer).await;
    }
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

/// Route a validated notification command to the paired native session.
/// A successful response means the command was accepted for delivery to the
/// phone, not that Android confirmed the dismissal, action, or reply. State
/// changes arrive later as ordinary notification events.
async fn handle_native_notification_command<W>(
    command: NotificationCommand,
    notification_id: handover_core::NotificationId,
    writer: &mut W,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let Some(native) = native_backend() else {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(
                ErrorCode::BackendUnavailable,
                "native backend unavailable",
            ),
        )
        .await?;
        return Ok(true);
    };
    let peer_id = notification_id
        .device_id
        .as_str()
        .strip_prefix("native:")
        .unwrap_or_default();
    match native.execute_notification(peer_id, &command) {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::CommandCompleted { notification_id }),
            )
            .await?;
        }
        Err(error) => {
            use handover_native::NativeCommandError;
            let (code, message) = match error {
                NativeCommandError::Offline => (
                    ErrorCode::DeviceDisconnected,
                    "native device is disconnected",
                ),
                NativeCommandError::QueueFull => (
                    ErrorCode::BackendRejected,
                    "native backend could not accept the command",
                ),
            };
            warn!(%notification_id, "native notification command not accepted");
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
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

    if let Some(peer_id) = device_id.as_str().strip_prefix("native:") {
        let result = native_backend()
            .ok_or(handover_native::NativeCommandError::Offline)
            .and_then(|native| {
                if is_file {
                    let path = Url::parse(&url)
                        .ok()
                        .and_then(|url| url.to_file_path().ok())
                        .ok_or(handover_native::NativeCommandError::QueueFull)?;
                    native.share_file(peer_id, path)
                } else {
                    native.share_url(peer_id, url)
                }
            });
        let payload = match result {
            Ok(transfer_id) => ServerMessage::new(ServerPayload::ShareAccepted {
                device_id,
                transfer_id: Some(transfer_id),
            }),
            Err(handover_native::NativeCommandError::Offline) => ServerMessage::protocol_error(
                ErrorCode::DeviceDisconnected,
                "native device is disconnected",
            ),
            Err(handover_native::NativeCommandError::QueueFull) => ServerMessage::protocol_error(
                ErrorCode::BackendRejected,
                "native backend could not accept the share",
            ),
        };
        write_json_line(writer, &payload).await?;
        return Ok(true);
    }

    match handover_kdeconnect::KdeConnectBackend::share_url(&device_id, &url).await {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::ShareAccepted {
                    device_id,
                    transfer_id: None,
                }),
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

async fn handle_messaging_send<W>(
    conversation_id: ConversationId,
    text: String,
    reply_to: Option<MessageId>,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let account = conversation_id.account_id.as_str().to_string();
    let conversation = conversation_id.local_id.clone();
    request_messaging(
        MessagingCommand::SendText {
            conversation_id,
            text: text.clone(),
            reply_to: reply_to.clone(),
        },
        move |request_id| handover_gmessages::contract::HelperCommand::SendText {
            request_id,
            account,
            conversation,
            text,
            reply_to: reply_to.map(|id| id.local_id),
        },
        writer,
        state,
        messaging,
    )
    .await
}

async fn handle_send_file<W>(
    conversation_id: ConversationId,
    file_url: String,
    caption: Option<String>,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let url = match validate_file_url(&file_url).await {
        Ok(url) => url,
        Err(code) => {
            write_json_line(writer, &ServerMessage::protocol_error(code, "invalid file")).await?;
            return Ok(true);
        }
    };
    let path = Url::parse(&url)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .and_then(|path| path.to_str().map(str::to_string));
    let Some(path) = path else {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(ErrorCode::InvalidResource, "invalid file"),
        )
        .await?;
        return Ok(true);
    };
    // Bound outbound attachments before the helper ever reads them.
    let oversized = std::fs::metadata(&path)
        .map(|metadata| {
            !metadata.is_file() || metadata.len() > handover_gmessages::staging::MAX_STAGED_BYTES
        })
        .unwrap_or(true);
    if oversized {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(
                ErrorCode::InvalidMessagingCommand,
                "file is not usable",
            ),
        )
        .await?;
        return Ok(true);
    }
    let account = conversation_id.account_id.as_str().to_string();
    let conversation = conversation_id.local_id.clone();
    request_messaging(
        MessagingCommand::SendMedia {
            conversation_id,
            file_url: url.clone(),
            caption: caption.clone(),
        },
        move |request_id| handover_gmessages::contract::HelperCommand::SendMedia {
            request_id,
            account,
            conversation,
            path,
            caption,
        },
        writer,
        state,
        messaging,
    )
    .await
}

async fn handle_react<W>(
    message_id: MessageId,
    emoji: String,
    add: bool,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let account = message_id.conversation_id.account_id.as_str().to_string();
    let conversation = message_id.conversation_id.local_id.clone();
    let message = message_id.local_id.clone();
    let command = if add {
        MessagingCommand::React {
            message_id: message_id.clone(),
            emoji: emoji.clone(),
        }
    } else {
        MessagingCommand::Unreact {
            message_id: message_id.clone(),
            emoji: emoji.clone(),
        }
    };
    request_messaging(
        command,
        move |request_id| handover_gmessages::contract::HelperCommand::React {
            request_id,
            account,
            conversation,
            message,
            emoji,
            add,
        },
        writer,
        state,
        messaging,
    )
    .await
}

async fn handle_mark_read<W>(
    conversation_id: ConversationId,
    message_id: Option<MessageId>,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    // Default to the newest stored message; the helper attests the effect.
    let resolved = message_id
        .as_ref()
        .map(|id| id.local_id.clone())
        .or_else(|| {
            state
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .messaging()
                .history(&conversation_id, 1, None)
                .ok()
                .and_then(|(page, _)| page.last().map(|message| message.id.local_id.clone()))
        });
    let account = conversation_id.account_id.as_str().to_string();
    let conversation = conversation_id.local_id.clone();
    fire_messaging(
        MessagingCommand::MarkRead {
            conversation_id: conversation_id.clone(),
            message_id: message_id.clone(),
        },
        handover_gmessages::contract::HelperCommand::MarkRead {
            account,
            conversation,
            message: resolved,
        },
        conversation_id,
        writer,
        state,
        messaging,
    )
    .await
}

async fn handle_typing<W>(
    conversation_id: ConversationId,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let account = conversation_id.account_id.as_str().to_string();
    let conversation = conversation_id.local_id.clone();
    fire_messaging(
        MessagingCommand::TypingStart {
            conversation_id: conversation_id.clone(),
        },
        handover_gmessages::contract::HelperCommand::Typing {
            account,
            conversation,
        },
        conversation_id,
        writer,
        state,
        messaging,
    )
    .await
}

async fn handle_delete_message<W>(
    message_id: MessageId,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let account = message_id.conversation_id.account_id.as_str().to_string();
    let conversation = message_id.conversation_id.local_id.clone();
    let message = message_id.local_id.clone();
    request_messaging(
        MessagingCommand::DeleteMessage {
            message_id: message_id.clone(),
        },
        move |request_id| handover_gmessages::contract::HelperCommand::DeleteMessage {
            request_id,
            account,
            conversation,
            message,
        },
        writer,
        state,
        messaging,
    )
    .await
}

async fn handle_open_conversation<W>(
    account_id: MessagingAccountId,
    addresses: Vec<String>,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let account = account_id.as_str().to_string();
    request_messaging(
        MessagingCommand::OpenConversation {
            account_id: account_id.clone(),
            addresses: addresses.clone(),
        },
        move |request_id| handover_gmessages::contract::HelperCommand::OpenConversation {
            request_id,
            account,
            addresses,
        },
        writer,
        state,
        messaging,
    )
    .await
}

async fn handle_sync<W>(
    account_id: MessagingAccountId,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let known = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .messaging()
        .account(&account_id)
        .is_some();
    if !known {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(
                ErrorCode::UnknownMessagingAccount,
                "messaging account is not known",
            ),
        )
        .await?;
        return Ok(true);
    }
    let hub = match require_messaging_hub(messaging) {
        Ok(hub) => hub,
        Err((code, message)) => {
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            return Ok(true);
        }
    };
    match hub
        .fire(handover_gmessages::contract::HelperCommand::Sync {
            account: account_id.as_str().into(),
        })
        .await
    {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::AccountAccepted { account_id }),
            )
            .await?;
            Ok(true)
        }
        Err(error) => {
            let (code, message) = helper_call_error(error);
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            Ok(true)
        }
    }
}

async fn handle_login<W>(
    account_id: MessagingAccountId,
    bundle_b64: String,
    writer: &mut W,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    if account_id.as_str().is_empty() || account_id.as_str().len() > 128 {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(
                ErrorCode::InvalidMessagingCommand,
                "invalid messaging account",
            ),
        )
        .await?;
        return Ok(true);
    }
    if crate::messaging_backend::validate_login_bundle(&bundle_b64).is_err() {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(
                ErrorCode::CredentialRejected,
                "credential bundle was rejected",
            ),
        )
        .await?;
        return Ok(true);
    }
    let hub = match require_messaging_hub(messaging) {
        Ok(hub) => hub,
        Err((code, message)) => {
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            return Ok(true);
        }
    };
    // The bundle travels the local socket and the local helper pipe only.
    // It is never logged, never stored by the daemon, and never argv.
    let account = account_id.as_str().to_string();
    match hub
        .fire(handover_gmessages::contract::HelperCommand::Login {
            account,
            bundle_b64,
        })
        .await
    {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::AccountAccepted { account_id }),
            )
            .await?;
            Ok(true)
        }
        Err(error) => {
            let (code, message) = helper_call_error(error);
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            Ok(true)
        }
    }
}

async fn handle_logout<W>(
    account_id: MessagingAccountId,
    writer: &mut W,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let hub = match require_messaging_hub(messaging) {
        Ok(hub) => hub,
        Err((code, message)) => {
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            return Ok(true);
        }
    };
    // Revocation happens helper-side (remote revoke plus local secret
    // deletion); the daemon drops its copy when AccountRemoved arrives.
    // Acceptance here means the request was queued, not that access is gone.
    match hub
        .fire(handover_gmessages::contract::HelperCommand::Logout {
            account: account_id.as_str().into(),
        })
        .await
    {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::AccountAccepted { account_id }),
            )
            .await?;
            Ok(true)
        }
        Err(error) => {
            let (code, message) = helper_call_error(error);
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            Ok(true)
        }
    }
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

fn route_call(
    state: &Arc<RwLock<StateStore>>,
    device_id: DeviceId,
    action: handover_core::CallAction,
    address: Option<String>,
) -> ServerPayload {
    let current = snapshot(state);
    let call = current
        .calls
        .iter()
        .find(|call| call.device_id == device_id);
    let allowed = current
        .devices
        .iter()
        .any(|d| d.id == device_id && d.connected && d.paired)
        && call.is_some_and(|call| call.controls.contains(&action))
        && match action {
            handover_core::CallAction::Place => address
                .as_deref()
                .is_some_and(handover_core::valid_call_address),
            _ => address.is_none(),
        };
    let generation = call.map(|call| call.generation);
    if !allowed {
        return ServerPayload::Error {
            code: ErrorCode::BackendRejected,
            message: "call action unavailable in current phone state or invalid address".into(),
        };
    }
    let result = device_id.as_str().strip_prefix("native:").and_then(|id| {
        native_backend().map(|native| native.call_control(id, action.as_str(), address, generation))
    });
    match result {
        Some(Ok(())) => ServerPayload::NativeAccepted,
        Some(Err(_)) => ServerPayload::Error {
            code: ErrorCode::BackendRejected,
            message: "call command could not be queued".into(),
        },
        None => ServerPayload::Error {
            code: ErrorCode::BackendUnavailable,
            message: "call backend unavailable".into(),
        },
    }
}

fn snapshot(state: &Arc<RwLock<StateStore>>) -> StateSnapshot {
    state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .snapshot()
}

struct MessagingSnapshot {
    accounts: Vec<handover_core::MessagingAccount>,
    conversations: Vec<handover_core::Conversation>,
    typing: Vec<handover_core::TypingState>,
    read: Vec<handover_core::ReadState>,
}

fn messaging_snapshot(state: &Arc<RwLock<StateStore>>) -> MessagingSnapshot {
    let guard = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    MessagingSnapshot {
        accounts: guard.messaging().snapshot_accounts(),
        conversations: guard.messaging().snapshot_conversations(),
        typing: guard.messaging().snapshot_typing(),
        read: guard.messaging().snapshot_read(),
    }
}

fn snapshot_payload(
    state: &Arc<RwLock<StateStore>>,
    include_media: bool,
    include_messages: bool,
) -> ServerPayload {
    let snapshot = snapshot(state);
    let messaging = messaging_snapshot(state);
    ServerPayload::Snapshot {
        devices: snapshot.devices,
        calls: snapshot.calls,
        notifications: snapshot.notifications,
        media_sessions: if include_media {
            snapshot.media_sessions
        } else {
            Vec::new()
        },
        messaging_accounts: if include_messages {
            messaging.accounts
        } else {
            Vec::new()
        },
        conversations: if include_messages {
            messaging.conversations
        } else {
            Vec::new()
        },
        typing_states: if include_messages {
            messaging.typing
        } else {
            Vec::new()
        },
        read_states: if include_messages {
            messaging.read
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
        StateEvent::Call(event) => ServerMessage::from_call_event(event),
        StateEvent::Messaging(event) => ServerMessage::from_messaging_event(event),
        StateEvent::ShareReceived(share) => {
            ServerMessage::new(ServerPayload::ShareReceived { share })
        }
        StateEvent::ShareResult(result) => {
            ServerMessage::new(ServerPayload::ShareResult { result })
        }
    }
}

fn messaging_validation_error(error: &MessagingValidationError) -> (ErrorCode, &'static str) {
    match error {
        MessagingValidationError::UnknownAccount => (
            ErrorCode::UnknownMessagingAccount,
            "messaging account is not known",
        ),
        MessagingValidationError::UnknownConversation => {
            (ErrorCode::UnknownConversation, "conversation is not known")
        }
        MessagingValidationError::UnknownMessage => {
            (ErrorCode::UnknownMessage, "message is not known")
        }
        MessagingValidationError::AccountUnavailable => (
            ErrorCode::MessagingUnavailable,
            "messaging account is unavailable",
        ),
        MessagingValidationError::Invalid(
            handover_core::ValidationError::UnsupportedCapability(_),
        ) => (
            ErrorCode::UnsupportedMessagingCapability,
            "conversation does not attest that capability",
        ),
        MessagingValidationError::Invalid(_) => (
            ErrorCode::InvalidMessagingCommand,
            "invalid messaging command",
        ),
    }
}

fn helper_call_error(error: HelperCallError) -> (ErrorCode, String) {
    match error {
        HelperCallError::Unavailable => (
            ErrorCode::MessagingUnavailable,
            "messaging helper is unavailable".into(),
        ),
        HelperCallError::Busy => (
            ErrorCode::BackendRejected,
            "messaging helper is busy".into(),
        ),
        HelperCallError::Timeout => (
            ErrorCode::BackendRejected,
            "messaging helper timed out".into(),
        ),
        HelperCallError::Rejected(reason) if !reason.is_empty() => {
            (ErrorCode::BackendRejected, reason)
        }
        HelperCallError::Rejected(_) => (
            ErrorCode::BackendRejected,
            "messaging helper rejected the command".into(),
        ),
        HelperCallError::BadBundle => (
            ErrorCode::CredentialRejected,
            "credential bundle was rejected".into(),
        ),
    }
}

fn require_messaging_hub(
    messaging: &Option<MessagingHub>,
) -> Result<&MessagingHub, (ErrorCode, String)> {
    messaging
        .as_ref()
        .ok_or_else(|| helper_call_error(HelperCallError::Unavailable))
}

async fn handle_conversations<W>(
    account_id: MessagingAccountId,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let (known, conversations) = {
        let guard = state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            guard.messaging().account(&account_id).is_some(),
            guard
                .messaging()
                .snapshot_conversations()
                .into_iter()
                .filter(|conversation| conversation.id.account_id == account_id)
                .collect::<Vec<_>>(),
        )
    };
    if !known {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(
                ErrorCode::UnknownMessagingAccount,
                "messaging account is not known",
            ),
        )
        .await?;
        return Ok(true);
    }
    write_json_line(
        writer,
        &ServerMessage::new(ServerPayload::Conversations { conversations }),
    )
    .await?;
    Ok(true)
}

async fn handle_history<W>(
    conversation_id: ConversationId,
    limit: Option<u32>,
    cursor: Option<String>,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let limit = limit.unwrap_or(25).clamp(1, 100) as usize;
    let read = || {
        state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .messaging()
            .history(&conversation_id, limit, cursor.as_deref())
    };
    match read() {
        Ok((mut messages, mut cursor_next)) => {
            // A successful local read may still be only a bounded live-event
            // window. If the caller asks for more than we currently hold,
            // give the helper one chance to fill the newest page before
            // declaring that history is exhausted. This is what makes a
            // larger "load all" request materially different from rereading
            // the same local window.
            if cursor.is_none() && limit == 100 && messages.len() < limit {
                if let Some(hub) = messaging {
                    if hub
                        .fetch_through_helper(
                            conversation_id.account_id.as_str(),
                            &conversation_id.local_id,
                            limit as u32,
                            None,
                        )
                        .await
                        .is_ok()
                    {
                        if let Ok((fetched_messages, fetched_cursor)) = read() {
                            messages = fetched_messages;
                            cursor_next = fetched_cursor;
                        }
                    }
                }
            }
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::History {
                    conversation_id,
                    messages,
                    cursor_next,
                }),
            )
            .await?;
            Ok(true)
        }
        Err(HistoryGap::UnknownConversation) => {
            write_json_line(
                writer,
                &ServerMessage::protocol_error(
                    ErrorCode::UnknownConversation,
                    "conversation is not known",
                ),
            )
            .await?;
            Ok(true)
        }
        Err(HistoryGap::CursorOutsideWindow) => {
            // The cursor aged out of the bounded window: page older history
            // through the helper, then serve from the merged window.
            let hub = match require_messaging_hub(messaging) {
                Ok(hub) => hub,
                Err((code, message)) => {
                    write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
                    return Ok(true);
                }
            };
            let fetched = hub
                .fetch_through_helper(
                    conversation_id.account_id.as_str(),
                    &conversation_id.local_id,
                    limit as u32,
                    cursor.clone(),
                )
                .await;
            if fetched.is_err() {
                write_json_line(
                    writer,
                    &ServerMessage::protocol_error(
                        ErrorCode::HistoryUnavailable,
                        "older history is unavailable",
                    ),
                )
                .await?;
                return Ok(true);
            }
            match read() {
                Ok((messages, cursor_next)) => {
                    write_json_line(
                        writer,
                        &ServerMessage::new(ServerPayload::History {
                            conversation_id,
                            messages,
                            cursor_next,
                        }),
                    )
                    .await?;
                    Ok(true)
                }
                Err(_) => {
                    write_json_line(
                        writer,
                        &ServerMessage::protocol_error(
                            ErrorCode::HistoryUnavailable,
                            "older history is unavailable",
                        ),
                    )
                    .await?;
                    Ok(true)
                }
            }
        }
    }
}

/// Validate a command, forward it for a `CommandResult` acceptance report,
/// and reply with the acceptance. Acceptance is never delivery.
async fn request_messaging<W>(
    command: MessagingCommand,
    build: impl FnOnce(String) -> handover_gmessages::contract::HelperCommand,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let validation = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .messaging()
        .validate_messaging_command(&command);
    if let Err(error) = validation {
        let (code, message) = messaging_validation_error(&error);
        write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
        return Ok(true);
    }
    let hub = match require_messaging_hub(messaging) {
        Ok(hub) => hub,
        Err((code, message)) => {
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            return Ok(true);
        }
    };
    match hub.request(build).await {
        Ok(outcome) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::MessageAccepted {
                    request_id: outcome.request_id,
                }),
            )
            .await?;
            Ok(true)
        }
        Err(error) => {
            let (code, message) = helper_call_error(error);
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            Ok(true)
        }
    }
}

/// Validate a fire-and-forget command (read receipts, typing pings) and
/// queue it for the helper. The reply confirms daemon acceptance only.
async fn fire_messaging<W>(
    command: MessagingCommand,
    helper: handover_gmessages::contract::HelperCommand,
    conversation_id: ConversationId,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let validation = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .messaging()
        .validate_messaging_command(&command);
    if let Err(error) = validation {
        let (code, message) = messaging_validation_error(&error);
        write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
        return Ok(true);
    }
    let hub = match require_messaging_hub(messaging) {
        Ok(hub) => hub,
        Err((code, message)) => {
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            return Ok(true);
        }
    };
    match hub.fire(helper).await {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::ConversationAccepted { conversation_id }),
            )
            .await?;
            Ok(true)
        }
        Err(error) => {
            let (code, message) = helper_call_error(error);
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            Ok(true)
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

    #[tokio::test]
    async fn calls_snapshot_and_rejections_use_isolated_ipc() {
        use handover_core::{CallAction, CallEvent, CallPhase, CallState};
        let (_directory, path, state, _events, task) = server_with_device().await;
        // No native backend or real phone exists in this fixture.
        let call = CallState {
            device_id: DeviceId::new("phone"),
            phase: CallPhase::Idle,
            controls: BTreeSet::from([CallAction::Place]),
            generation: 1,
        };
        state
            .write()
            .unwrap()
            .apply(StateEvent::Call(CallEvent::Updated(call.clone())));
        let mut client = Client::connect_to(path.clone()).await.unwrap();
        assert_eq!(client.calls().await.unwrap(), vec![call.clone()]);
        assert!(
            client
                .call_control(call.device_id.clone(), CallAction::Answer, None)
                .await
                .is_err()
        );
        assert!(
            client
                .call_control(
                    call.device_id.clone(),
                    CallAction::Place,
                    Some("*#06#".into())
                )
                .await
                .is_err()
        );
        let subscription = client.subscribe().await.unwrap();
        assert_eq!(subscription.calls, vec![call]);
        task.abort();
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
        let server = IpcServer::bind_at(path.clone(), Arc::clone(&state), events.clone(), None)
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
    async fn native_notification_commands_route_away_from_kde() {
        use handover_core::{Notification, NotificationId};
        let (_directory, path, state, _events, task) = server_with_device().await;
        let native = Notification {
            id: NotificationId::new(handover_core::DeviceId::new("native:cert"), "key-1"),
            app_name: "Example".into(),
            title: "Hello".into(),
            body: "World".into(),
            icon_path: None,
            clearable: true,
            actions: vec![],
            reply_supported: false,
        };
        state
            .write()
            .unwrap()
            .apply(StateEvent::Notification(NotificationEvent::Added(
                native.clone(),
            )));
        let mut client = Client::connect_to(path).await.expect("connect");
        // No native backend is running in this test, so routing must report
        // an unavailable native path rather than attempting a KDE D-Bus call
        // for a `native:` device.
        let error = client
            .dismiss_notification(native.id)
            .await
            .expect_err("native backend absent");
        assert!(matches!(
            error,
            IpcError::Server {
                code: ErrorCode::BackendUnavailable,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn native_media_commands_route_away_from_kde() {
        use handover_core::{MediaSession, MediaSessionId, PlaybackState};
        let (_directory, path, state, _events, task) = server_with_device().await;
        // The fixture device lacks the media capability, so add a capable
        // native device first: routing is checked after validation passes.
        let device = Device {
            id: DeviceId::new("native:cert"),
            name: "Native Phone".into(),
            connected: true,
            paired: true,
            battery: None,
            capabilities: BTreeSet::from([
                handover_core::Capability::Battery,
                handover_core::Capability::Media,
            ]),
        };
        state
            .write()
            .unwrap()
            .apply(StateEvent::Device(DeviceEvent::Added(device.clone())));
        let session = MediaSession {
            id: MediaSessionId::new(device.id.clone(), "com.example.music"),
            application: "Example Music".into(),
            title: Some("Test track".into()),
            artist: None,
            album: None,
            playback: PlaybackState::Playing,
            position_ms: None,
            duration_ms: None,
            volume_percent: None,
            controls: BTreeSet::from([handover_core::MediaControl::Pause]),
        };
        state
            .write()
            .unwrap()
            .apply(StateEvent::Media(MediaEvent::Added(session.clone())));
        let mut client = Client::connect_to(path).await.expect("connect");
        // No native backend is running in this test, so routing must report
        // an unavailable native path rather than attempting a KDE D-Bus call
        // for a `native:` session.
        let error = client
            .media_command(MediaCommand::Pause { id: session.id })
            .await
            .expect_err("native backend absent");
        assert!(matches!(
            error,
            IpcError::Server {
                code: ErrorCode::BackendUnavailable,
                ..
            }
        ));
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
            snapshot_payload(&state, true, false),
            ServerPayload::Snapshot {
                devices: vec![device("Phone", 72)],
                notifications: vec![current],
                media_sessions: vec![media],
                calls: vec![],
                messaging_accounts: vec![],
                conversations: vec![],
                typing_states: vec![],
                read_states: vec![],
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

#[cfg(test)]
mod messaging_live_tests {
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};
    use std::time::{Duration, Instant};

    use handover_core::{ConversationId, MessagingAccountId};
    use handover_ipc::{Client, ErrorCode, IpcError, ServerPayload};
    use tokio::time::sleep;

    use super::*;
    use crate::messaging_backend::{MessagingHub, spawn_supervisor};

    fn live_device(name: &str, percentage: u8) -> handover_core::Device {
        handover_core::Device {
            id: handover_core::DeviceId::new(name.to_lowercase()),
            name: name.into(),
            connected: true,
            paired: true,
            battery: Some(
                handover_core::BatteryState::new(percentage, false).expect("valid battery"),
            ),
            capabilities: std::collections::BTreeSet::from([handover_core::Capability::Battery]),
        }
    }

    fn helper_binary() -> Option<PathBuf> {
        for candidate in [
            PathBuf::from("../target/debug/handover-gmessages-helper"),
            PathBuf::from("target/debug/handover-gmessages-helper"),
        ] {
            if candidate.is_file() {
                return candidate.canonicalize().ok();
            }
        }
        None
    }

    async fn wait_until(mut condition: impl FnMut() -> bool, timeout: Duration, what: &str) {
        let start = Instant::now();
        while !condition() {
            if start.elapsed() > timeout {
                panic!("timed out waiting for {what}");
            }
            sleep(Duration::from_millis(50)).await;
        }
    }

    /// Full live walkthrough against the in-memory loopback helper: login,
    /// seeded SMS/RCS/group conversations, paged history, send, attachment,
    /// reply, reactions, typing, read state, delivery progression, open,
    /// delete, restart recovery, isolation, and logout.
    #[tokio::test]
    async fn messaging_end_to_end_through_loopback_helper() {
        let Some(helper) = helper_binary() else {
            eprintln!("skipping live messaging test: helper binary not built");
            return;
        };
        let state_home = tempfile::tempdir().expect("state home");
        // Test-only env mutation. No other test in this binary reads these
        // variables, and the helper child inherits them for secret storage.
        unsafe {
            std::env::set_var("HANDOVER_GMESSAGES_HELPER", &helper);
            std::env::set_var("XDG_STATE_HOME", state_home.path());
        }

        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("handoverd.sock");
        let mut store = StateStore::default();
        store.apply(StateEvent::Device(DeviceEvent::Added(live_device(
            "Phone", 72,
        ))));
        let state = Arc::new(RwLock::new(store));
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let hub = MessagingHub::new();
        let supervisor = spawn_supervisor(Arc::clone(&state), events.clone(), hub.clone());
        let server = IpcServer::bind_at(
            path.clone(),
            Arc::clone(&state),
            events.clone(),
            Some(hub.clone()),
        )
        .await
        .expect("server binds");
        let task = tokio::spawn(async move {
            server.run().await.expect("server runs");
        });

        let mut client = Client::connect_to(path.clone()).await.expect("connect");
        let subscriber = Client::connect_to(path).await.expect("connect");
        let mut subscription = subscriber.subscribe().await.expect("subscribe");

        // The supervisor spawns asynchronously; wait for the helper link.
        wait_until(
            || hub.try_has_sender(),
            Duration::from_secs(10),
            "helper link",
        )
        .await;

        // Empty bundles are rejected before touching the helper.
        let login_error = client
            .messaging_login(MessagingAccountId::new("gmessages:test"), String::new())
            .await
            .expect_err("empty bundle");
        assert!(matches!(
            login_error,
            IpcError::Server {
                code: ErrorCode::CredentialRejected,
                ..
            }
        ));

        // Login queues the bundle with the helper; pairing completes on sync.
        client
            .messaging_login(MessagingAccountId::new("gmessages:test"), "dGVzdA==".into())
            .await
            .expect("login accepted");
        wait_until(
            || {
                state
                    .read()
                    .unwrap()
                    .messaging()
                    .account(&MessagingAccountId::new("gmessages:test"))
                    .is_some_and(|account| account.authenticated)
            },
            Duration::from_secs(15),
            "authenticated account",
        )
        .await;

        // Seeded SMS, RCS, and group conversations load.
        let conversations = client
            .messaging_conversations(MessagingAccountId::new("gmessages:test"))
            .await
            .expect("conversations");
        assert_eq!(conversations.len(), 3);
        let rcs = conversations
            .iter()
            .find(|conversation| conversation.id.local_id == "thread-rcs")
            .expect("rcs thread");
        assert_eq!(rcs.transport, handover_core::TransportKind::Rcs);
        assert!(
            rcs.capabilities
                .contains(&handover_core::MessagingCapability::Text)
        );
        let group = conversations
            .iter()
            .find(|conversation| conversation.id.local_id == "thread-group")
            .expect("group thread");
        assert_eq!(group.kind, handover_core::ConversationKind::Group);
        assert_eq!(group.participants.len(), 3);

        // Paged history: newest page plus an older cursor page.
        let (page, next) = client
            .messaging_history(rcs.id.clone(), Some(2), None)
            .await
            .expect("history");
        assert_eq!(page.len(), 2);
        let next = next.expect("older page exists");
        let (older, end) = client
            .messaging_history(rcs.id.clone(), Some(10), Some(next))
            .await
            .expect("older history");
        assert!(!older.is_empty());
        assert!(end.is_none());

        // Unknown conversations stay unknown; bogus cursors report a gap.
        let unknown = client
            .messaging_history(
                ConversationId::new(MessagingAccountId::new("gmessages:test"), "missing"),
                None,
                None,
            )
            .await
            .expect_err("unknown conversation");
        assert!(matches!(
            unknown,
            IpcError::Server {
                code: ErrorCode::UnknownConversation,
                ..
            }
        ));
        let gap = client
            .messaging_history(rcs.id.clone(), None, Some("missing-cursor".into()))
            .await
            .expect_err("cursor gap");
        assert!(matches!(
            gap,
            IpcError::Server {
                code: ErrorCode::HistoryUnavailable,
                ..
            }
        ));

        // Reactions need an attested capability: the SMS thread has none.
        let sms = conversations
            .iter()
            .find(|conversation| conversation.id.local_id == "thread-sms")
            .expect("sms thread");
        let (sms_history, _) = client
            .messaging_history(sms.id.clone(), Some(1), None)
            .await
            .expect("sms history");
        let sms_message = sms_history.first().expect("sms message").id.clone();
        let react_error = client
            .react_to_message(sms_message, "❤".into())
            .await
            .expect_err("sms reactions unattested");
        assert!(matches!(
            react_error,
            IpcError::Server {
                code: ErrorCode::UnsupportedMessagingCapability,
                ..
            }
        ));

        // Send text: accepted now, displayed later, never assumed.
        let request_id = client
            .send_message_text(rcs.id.clone(), "hello from the live test".into())
            .await
            .expect("send accepted");
        assert!(request_id.starts_with("msgreq-"));
        // Drain the attested pipeline one stage per sync.
        for _ in 0..3 {
            client
                .messaging_sync(MessagingAccountId::new("gmessages:test"))
                .await
                .expect("sync");
        }
        let displayed = wait_for_status(&mut subscription, "displayed").await;
        assert!(displayed.starts_with("gmessages:test:thread-rcs:out-"));

        // The sent message is visible with self attribution.
        let (fresh, _) = client
            .messaging_history(rcs.id.clone(), Some(10), None)
            .await
            .expect("fresh history");
        let own = fresh
            .iter()
            .find(|message| {
                message.sender.is_self
                    && message.text.as_deref() == Some("hello from the live test")
            })
            .expect("own message stored")
            .id
            .clone();

        // Reply sends into the same thread (validated by the daemon).
        client
            .send_message_text(rcs.id.clone(), "a reply in the same thread".into())
            .await
            .expect("reply accepted");

        // Attachment round trip with a real staged file.
        let attachment_dir = tempfile::tempdir().expect("attachment dir");
        let attachment_path = attachment_dir.path().join("live ✓.bin");
        tokio::fs::write(&attachment_path, b"live attachment bytes")
            .await
            .expect("write attachment");
        let file_url = url::Url::from_file_path(&attachment_path)
            .expect("file url")
            .to_string();
        client
            .send_message_file(rcs.id.clone(), file_url, Some("caption".into()))
            .await
            .expect("attachment accepted");
        let (with_file, _) = client
            .messaging_history(rcs.id.clone(), Some(10), None)
            .await
            .expect("history with file");
        let staged = with_file
            .iter()
            .find(|message| {
                message
                    .attachments
                    .iter()
                    .any(|attachment| attachment.name.as_deref() == Some("live ✓.bin"))
            })
            .expect("attachment stored");
        assert_eq!(
            staged.attachments[0].staged_path.as_deref(),
            attachment_path.to_str()
        );

        // Reactions add and remove.
        client
            .react_to_message(own.clone(), "👍".into())
            .await
            .expect("react accepted");
        let (reacted, _) = client
            .messaging_history(rcs.id.clone(), Some(10), None)
            .await
            .expect("reacted history");
        let entry = reacted
            .iter()
            .find(|message| message.id == own)
            .expect("own message");
        assert!(
            entry
                .reactions
                .iter()
                .any(|reaction| reaction.emoji == "👍")
        );
        client
            .unreact_to_message(own.clone(), "👍".into())
            .await
            .expect("unreact accepted");
        let (unreacted, _) = client
            .messaging_history(rcs.id.clone(), Some(10), None)
            .await
            .expect("unreacted history");
        let entry = unreacted
            .iter()
            .find(|message| message.id == own)
            .expect("own message");
        assert!(entry.reactions.is_empty());

        // Only own messages can be deleted; peer messages are rejected.
        let peer = fresh
            .iter()
            .find(|message| !message.sender.is_self)
            .expect("peer message")
            .id
            .clone();
        let delete_error = client
            .delete_message(peer)
            .await
            .expect_err("peer delete rejected");
        assert!(matches!(
            delete_error,
            IpcError::Server {
                code: ErrorCode::InvalidMessagingCommand,
                ..
            }
        ));
        client
            .delete_message(own.clone())
            .await
            .expect("delete accepted");
        let (after_delete, _) = client
            .messaging_history(rcs.id.clone(), Some(20), None)
            .await
            .expect("history after delete");
        assert!(!after_delete.iter().any(|message| message.id == own));

        // Typing-start queues; the helper reports inbound typing.
        client
            .start_typing(rcs.id.clone())
            .await
            .expect("typing accepted");
        wait_for_typing(&mut subscription, &rcs.id).await;

        // Mark read clears the unread flag.
        client
            .mark_conversation_read(rcs.id.clone(), None)
            .await
            .expect("read accepted");
        wait_until(
            || {
                state
                    .read()
                    .unwrap()
                    .messaging()
                    .snapshot_read()
                    .iter()
                    .any(|read| read.conversation_id == rcs.id && !read.unread)
            },
            Duration::from_secs(10),
            "read state",
        )
        .await;

        // Open a new conversation by address.
        client
            .open_conversation(
                MessagingAccountId::new("gmessages:test"),
                vec!["+15559999".into()],
            )
            .await
            .expect("open accepted");
        wait_until(
            || {
                state
                    .read()
                    .unwrap()
                    .messaging()
                    .snapshot_conversations()
                    .len()
                    == 4
            },
            Duration::from_secs(10),
            "opened conversation",
        )
        .await;

        // Helper traffic never disturbs device state.
        assert_eq!(client.devices().await.expect("devices").len(), 1);

        // Logout revokes access: the account and its state disappear.
        client
            .messaging_logout(MessagingAccountId::new("gmessages:test"))
            .await
            .expect("logout accepted");
        wait_until(
            || {
                state
                    .read()
                    .unwrap()
                    .messaging()
                    .snapshot_accounts()
                    .is_empty()
            },
            Duration::from_secs(10),
            "account removal",
        )
        .await;
        assert!(
            state
                .read()
                .unwrap()
                .messaging()
                .snapshot_conversations()
                .is_empty()
        );

        // Shut the helper down so no orphan process outlives the test:
        // aborting the supervisor drops the child (kill-on-drop).
        supervisor.abort();
        task.abort();
        unsafe {
            std::env::remove_var("HANDOVER_GMESSAGES_HELPER");
            std::env::remove_var("XDG_STATE_HOME");
        }
    }

    async fn wait_for_status(
        subscription: &mut handover_ipc::Subscription,
        status: &str,
    ) -> String {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if Instant::now() > deadline {
                panic!("timed out waiting for status {status}");
            }
            let message = tokio::time::timeout(Duration::from_secs(5), subscription.next_message())
                .await
                .expect("event in time")
                .expect("event decodes");
            if let ServerPayload::MessageStatus { update } = message.payload {
                let rendered = format!("{:?}", update.status).to_lowercase();
                if rendered.contains(status) {
                    return update.message_id.to_string();
                }
            }
        }
    }

    async fn wait_for_typing(
        subscription: &mut handover_ipc::Subscription,
        conversation: &ConversationId,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if Instant::now() > deadline {
                panic!("timed out waiting for typing");
            }
            let message = tokio::time::timeout(Duration::from_secs(5), subscription.next_message())
                .await
                .expect("event in time")
                .expect("event decodes");
            if let ServerPayload::Typing { state } = message.payload {
                if &state.conversation_id == conversation && !state.participant_ids.is_empty() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod messaging_failure_tests {
    use std::sync::{Arc, RwLock};

    use handover_core::{
        BatteryState, Capability, Conversation, ConversationEvent, ConversationId,
        ConversationKind, Device, DeviceId, Message, MessageId, MessagingAccount,
        MessagingAccountEvent, MessagingAccountId, MessagingEvent as CoreEvent, Participant,
        TransportKind,
    };
    use handover_ipc::{Client, ErrorCode, IpcError};
    use std::collections::BTreeSet;

    use super::*;

    fn seeded_state() -> Arc<RwLock<StateStore>> {
        let account_id = MessagingAccountId::new("gmessages:test");
        let conversation_id = ConversationId::new(account_id.clone(), "thread-1");
        let mut store = StateStore::default();
        store.apply(StateEvent::Messaging(CoreEvent::Account(
            MessagingAccountEvent::Added(MessagingAccount {
                id: account_id.clone(),
                label: "Test".into(),
                connected: true,
                authenticated: true,
            }),
        )));
        store.apply(StateEvent::Messaging(CoreEvent::Conversation(
            ConversationEvent::Added(Conversation {
                id: conversation_id.clone(),
                kind: ConversationKind::Direct,
                transport: TransportKind::Rcs,
                title: None,
                participants: vec![
                    Participant {
                        local_id: "self".into(),
                        display_name: None,
                        address: None,
                        is_self: true,
                    },
                    Participant {
                        local_id: "peer".into(),
                        display_name: None,
                        address: None,
                        is_self: false,
                    },
                ],
                latest_message_id: Some(MessageId::new(conversation_id.clone(), "m1")),
                last_activity_at: None,
                unread_count: None,
                cursor: None,
                capabilities: BTreeSet::from([handover_core::MessagingCapability::Text]),
            }),
        )));
        store.apply(StateEvent::Messaging(CoreEvent::Message(
            handover_core::MessageEvent::Added(Message {
                id: MessageId::new(conversation_id, "m1"),
                sender: Participant {
                    local_id: "peer".into(),
                    display_name: None,
                    address: None,
                    is_self: false,
                },
                transport: None,
                sent_at: Some(10),
                text: Some("hello".into()),
                attachments: vec![],
                reply_to: None,
                reactions: vec![],
                deleted: false,
            }),
        )));
        Arc::new(RwLock::new(store))
    }

    async fn server_without_helper(
        state: Arc<RwLock<StateStore>>,
    ) -> (tempfile::TempDir, PathBuf, tokio::task::JoinHandle<()>) {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("handoverd.sock");
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let server = IpcServer::bind_at(path.clone(), Arc::clone(&state), events, None)
            .await
            .expect("server binds");
        let task = tokio::spawn(async move {
            server.run().await.expect("server runs");
        });
        (directory, path, task)
    }

    #[tokio::test]
    async fn helper_down_reports_unavailable_without_touching_devices() {
        let state = seeded_state();
        state
            .write()
            .unwrap()
            .apply(StateEvent::Device(DeviceEvent::Added(Device {
                id: DeviceId::new("phone"),
                name: "Phone".into(),
                connected: true,
                paired: true,
                battery: Some(BatteryState::new(72, false).expect("valid")),
                capabilities: BTreeSet::from([Capability::Battery]),
            })));
        let (_directory, path, task) = server_without_helper(state).await;
        let mut client = Client::connect_to(path).await.expect("connect");

        // Devices keep working with no helper configured.
        assert_eq!(client.devices().await.expect("devices").len(), 1);

        // Validation still runs first: unknown targets report as unknown.
        let unknown = client
            .send_message_text(
                ConversationId::new(MessagingAccountId::new("gmessages:test"), "missing"),
                "hi".into(),
            )
            .await
            .expect_err("unknown conversation");
        assert!(matches!(
            unknown,
            IpcError::Server {
                code: ErrorCode::UnknownConversation,
                ..
            }
        ));

        // Known targets fail closed with MessagingUnavailable, never with a
        // guessed acceptance.
        let down = client
            .send_message_text(
                ConversationId::new(MessagingAccountId::new("gmessages:test"), "thread-1"),
                "hi".into(),
            )
            .await
            .expect_err("helper down");
        assert!(matches!(
            down,
            IpcError::Server {
                code: ErrorCode::MessagingUnavailable,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn login_rejects_bad_bundles_before_helper_contact() {
        let (_directory, path, task) = server_without_helper(seeded_state()).await;
        let mut client = Client::connect_to(path).await.expect("connect");
        let error = client
            .messaging_login(MessagingAccountId::new("gmessages:test"), String::new())
            .await
            .expect_err("empty bundle");
        assert!(matches!(
            error,
            IpcError::Server {
                code: ErrorCode::CredentialRejected,
                ..
            }
        ));
        // An oversized bundle cannot even cross the 64 KiB IPC line limit:
        // it fails closed at the transport (line-too-long, a closed
        // connection, or a broken pipe on the racing write), never reaching
        // the helper.
        let error = client
            .messaging_login(
                MessagingAccountId::new("gmessages:test"),
                "x".repeat(256 * 1024 + 1),
            )
            .await
            .expect_err("oversized bundle");
        assert!(
            matches!(
                error,
                IpcError::LineTooLong
                    | IpcError::ConnectionClosed
                    | IpcError::Server { .. }
                    | IpcError::Io(_)
            ),
            "oversized bundle must fail closed, got {error:?}"
        );
        task.abort();
    }

    #[tokio::test]
    async fn history_gap_without_helper_is_unavailable() {
        let (_directory, path, task) = server_without_helper(seeded_state()).await;
        let mut client = Client::connect_to(path).await.expect("connect");
        // No helper is configured, so an aged-out cursor cannot be paged:
        // the subsystem reports itself unavailable rather than guessing.
        let error = client
            .messaging_history(
                ConversationId::new(MessagingAccountId::new("gmessages:test"), "thread-1"),
                None,
                Some("aged-out".into()),
            )
            .await
            .expect_err("cursor gap");
        assert!(matches!(
            error,
            IpcError::Server {
                code: ErrorCode::MessagingUnavailable,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn unknown_account_stays_unknown() {
        let (_directory, path, task) = server_without_helper(seeded_state()).await;
        let mut client = Client::connect_to(path).await.expect("connect");
        let error = client
            .messaging_conversations(MessagingAccountId::new("gmessages:ghost"))
            .await
            .expect_err("unknown account");
        assert!(matches!(
            error,
            IpcError::Server {
                code: ErrorCode::UnknownMessagingAccount,
                ..
            }
        ));
        task.abort();
    }
}
