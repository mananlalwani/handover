use std::fs::{self, Permissions};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use handover_core::{Capability, DeviceId, MediaCommand, NotificationCommand, StateEvent};
use handover_ipc::{
    ErrorCode, IpcError, Method, PROTOCOL_VERSION, Request, ServerMessage, ServerPayload,
    read_json_line, runtime_directory, socket_path, write_json_line,
};
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncWrite, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Semaphore, broadcast};
use tokio::time::{Duration, timeout};
use tracing::{debug, warn};
use url::Url;

use crate::messaging::{HistoryGap, MessagingValidationError};
use crate::messaging_backend::{HelperCallError, MessagingHub};
use crate::state::{CommandValidationError, MediaValidationError, StateSnapshot, StateStore};
use crate::{apply_backend_event, native_backend};
use handover_core::DeviceEvent;
use handover_core::{ConversationId, MessageId, MessagingAccountId, MessagingCommand};

mod calls;
mod connection;
mod devices;
mod media;
mod messaging;
mod notifications;
mod protocol;
mod transfers;

use calls::*;
use connection::*;
use devices::*;
use media::*;
use messaging::*;
use notifications::*;
use protocol::*;
use transfers::*;

pub(crate) const EVENT_CAPACITY: usize = 64;
pub(crate) const MAX_CLIENTS: usize = 64;
pub(crate) const PRE_SUBSCRIPTION_TIMEOUT: Duration = Duration::from_secs(15);
/// Upper bound for one history response line. Readers reject lines
/// over 1 MiB, so a legal page (100 messages of up to 8,000
/// four-byte characters) must be cut to fit before writing. The
/// cursor always advances to the oldest kept message, so paging
/// overlaps instead of gapping.
pub(crate) const MAX_HISTORY_LINE_BYTES: usize = 768 * 1024;

/// Cut a history page to the line budget, newest messages first. The
/// returned cursor addresses the oldest kept message, so the next
/// page overlaps the cut instead of skipping it.
pub(crate) fn fit_history_page(
    conversation_id: &ConversationId,
    mut messages: Vec<handover_core::Message>,
    mut cursor_next: Option<String>,
) -> (Vec<handover_core::Message>, Option<String>) {
    while messages.len() > 1 {
        let probe = serde_json::to_vec(&ServerMessage::new(ServerPayload::History {
            conversation_id: conversation_id.clone(),
            messages: messages.clone(),
            cursor_next: cursor_next.clone(),
        }))
        .map(|line| line.len())
        .unwrap_or(usize::MAX);
        if probe <= MAX_HISTORY_LINE_BYTES {
            break;
        }
        messages.remove(0);
        cursor_next = messages.first().map(|message| message.id.local_id.clone());
    }
    (messages, cursor_next)
}

#[derive(Default)]
pub(crate) struct SubscriptionFlags {
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
    client_slots: Arc<Semaphore>,
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
            client_slots: Arc::new(Semaphore::new(MAX_CLIENTS)),
        })
    }

    pub(crate) async fn run(&self) -> Result<(), ServerError> {
        let mut retry_delay = Duration::from_millis(50);
        loop {
            let (stream, _address) = match self.listener.accept().await {
                Ok(connection) => {
                    retry_delay = Duration::from_millis(50);
                    connection
                }
                Err(error) => {
                    warn!(%error, "IPC accept failed; retrying");
                    tokio::time::sleep(retry_delay).await;
                    retry_delay = (retry_delay * 2).min(Duration::from_secs(1));
                    continue;
                }
            };
            let state = Arc::clone(&self.state);
            let events = self.events.clone();
            let messaging = self.messaging.clone();
            let client_slots = Arc::clone(&self.client_slots);
            let Ok(client_slot) = client_slots.try_acquire_owned() else {
                debug!("IPC client limit reached; rejecting connection");
                continue;
            };
            tokio::spawn(async move {
                let _client_slot = client_slot;
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
        Method::NativePing { id } => native_command_response(
            native_backend().map(|native| native.ping(&id)),
            "native ping",
        ),
        Method::NativeRing { id } => native_command_response(
            native_backend().map(|native| native.ring(&id)),
            "native ring",
        ),
        Method::NativeLock { id } => native_command_response(
            native_backend().map(|native| native.lock_device(&id)),
            "native lock",
        ),
        Method::NativeKeepAwake { id, inhibit } => native_command_response(
            native_backend().map(|native| native.keep_awake(&id, inhibit)),
            "native keep-awake",
        ),
        Method::NativeTethering { id } => native_command_response(
            native_backend().map(|native| native.tethering_settings(&id)),
            "native tethering",
        ),
        Method::NativeFilesystemList { id, path } => native_command_response(
            native_backend().map(|native| native.filesystem_list(&id, path).map(|_| ())),
            "native filesystem listing",
        ),
        Method::ScreensaverInhibit => {
            crate::set_manual_screensaver(Some(true));
            crate::refresh_screensaver(state);
            ServerPayload::NativeAccepted
        }
        Method::ScreensaverRelease => {
            crate::set_manual_screensaver(Some(false));
            crate::refresh_screensaver(state);
            ServerPayload::NativeAccepted
        }
        Method::ScreensaverFollow => {
            crate::set_manual_screensaver(None);
            crate::refresh_screensaver(state);
            ServerPayload::NativeAccepted
        }
        Method::RemoteInputSend {
            device_id,
            action,
            delta_x,
            delta_y,
            button,
            text,
        } => {
            let peer_id = device_id
                .as_str()
                .strip_prefix("native:")
                .unwrap_or_default();
            native_command_response(
                native_backend().map(|native| {
                    native.remote_input(
                        peer_id,
                        &handover_core::RemoteInputCommand {
                            device_id: device_id.clone(),
                            action,
                            delta_x,
                            delta_y,
                            button,
                            text,
                        },
                    )
                }),
                "remote input",
            )
        }
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
        Method::NotificationSend {
            device_id,
            app,
            title,
            body,
        } => {
            let peer_id = device_id
                .as_str()
                .strip_prefix("native:")
                .unwrap_or_default();
            match native_backend().map(|native| native.send_notification(peer_id, app, title, body))
            {
                Some(Ok(())) => ServerPayload::NativeAccepted,
                Some(Err(error)) => ServerPayload::Error {
                    code: match error {
                        handover_native::NativeCommandError::Offline => {
                            ErrorCode::DeviceDisconnected
                        }
                        handover_native::NativeCommandError::QueueFull => {
                            ErrorCode::BackendRejected
                        }
                    },
                    message: "notification was not accepted".into(),
                },
                None => ServerPayload::Error {
                    code: ErrorCode::BackendUnavailable,
                    message: "native backend unavailable".into(),
                },
            }
        }
        Method::ClipboardSend {
            device_id,
            text,
            html,
            uri,
        } => {
            let peer_id = device_id
                .as_str()
                .strip_prefix("native:")
                .unwrap_or_default();
            match native_backend().map(|native| native.clipboard_set(peer_id, &text, html, uri)) {
                Some(Ok(())) => ServerPayload::NativeAccepted,
                Some(Err(error)) => ServerPayload::Error {
                    code: match error {
                        handover_native::NativeCommandError::Offline => {
                            ErrorCode::DeviceDisconnected
                        }
                        handover_native::NativeCommandError::QueueFull => {
                            ErrorCode::BackendRejected
                        }
                    },
                    message: "clipboard was not accepted".into(),
                },
                None => ServerPayload::Error {
                    code: ErrorCode::BackendUnavailable,
                    message: "native backend unavailable".into(),
                },
            }
        }
        Method::ClipboardMirror { enable } => match crate::mirror() {
            Some(mirror) => {
                mirror.set_enabled(enable);
                ServerPayload::NativeAccepted
            }
            None => ServerPayload::Error {
                code: ErrorCode::BackendUnavailable,
                message: "clipboard mirroring is unavailable".into(),
            },
        },
        Method::ClipboardMirrorStatus => ServerPayload::ClipboardMirror {
            enabled: crate::mirror().is_some_and(|mirror| mirror.enabled()),
        },
        Method::ClipboardHistoryList => ServerPayload::ClipboardHistory {
            entries: crate::clipboard_history().entries(),
        },
        Method::ClipboardHistoryPin { id, pinned } => {
            match crate::clipboard_history().set_pinned(id, pinned) {
                Ok(true) => ServerPayload::NativeAccepted,
                Ok(false) => ServerPayload::Error {
                    code: ErrorCode::ResourceNotFound,
                    message: "clipboard history entry was not found".into(),
                },
                Err(_) => ServerPayload::Error {
                    code: ErrorCode::BackendRejected,
                    message: "clipboard history could not be saved".into(),
                },
            }
        }
        Method::ClipboardHistorySave { text } => {
            match crate::clipboard_history().save_pinned(&text) {
                Ok(()) => ServerPayload::NativeAccepted,
                Err(error) => ServerPayload::Error {
                    code: if error.kind() == std::io::ErrorKind::InvalidInput {
                        ErrorCode::InvalidResource
                    } else {
                        ErrorCode::BackendRejected
                    },
                    message: error.to_string(),
                },
            }
        }
        Method::ClipboardHistoryCopy { id } => match crate::clipboard_history().text(id) {
            Some(text) => match crate::clipboard::apply_plain(&text) {
                Ok(()) => ServerPayload::NativeAccepted,
                Err(_) => ServerPayload::Error {
                    code: ErrorCode::BackendRejected,
                    message: "clipboard history entry could not be copied".into(),
                },
            },
            None => ServerPayload::Error {
                code: ErrorCode::ResourceNotFound,
                message: "clipboard history entry was not found".into(),
            },
        },
        Method::ClipboardHistoryClear { include_pinned } => {
            match crate::clipboard_history().clear(include_pinned) {
                Ok(()) => ServerPayload::NativeAccepted,
                Err(_) => ServerPayload::Error {
                    code: ErrorCode::BackendRejected,
                    message: "clipboard history could not be cleared".into(),
                },
            }
        }
        Method::ClipboardSendCurrent { device_id } => match read_wayland_clipboard().await {
            Ok(WaylandClipboard::Text { text, html, uri }) => {
                let peer_id = device_id
                    .as_str()
                    .strip_prefix("native:")
                    .unwrap_or_default();
                match native_backend().map(|native| native.clipboard_set(peer_id, &text, html, uri))
                {
                    Some(Ok(())) => ServerPayload::NativeAccepted,
                    Some(Err(_)) => ServerPayload::Error {
                        code: ErrorCode::BackendRejected,
                        message: "clipboard was not accepted".into(),
                    },
                    None => ServerPayload::Error {
                        code: ErrorCode::BackendUnavailable,
                        message: "native backend unavailable".into(),
                    },
                }
            }
            Ok(WaylandClipboard::File { path, mime }) => {
                let peer_id = device_id
                    .as_str()
                    .strip_prefix("native:")
                    .unwrap_or_default();
                // clipboard_file takes ownership of the temp on every
                // outcome, but a missing backend never sees it.
                match native_backend() {
                    None => {
                        let _ = std::fs::remove_file(&path);
                        ServerPayload::Error {
                            code: ErrorCode::BackendUnavailable,
                            message: "native backend unavailable".into(),
                        }
                    }
                    Some(native) => match native.clipboard_file(peer_id, path, mime) {
                        Ok(_) => ServerPayload::NativeAccepted,
                        Err(_) => ServerPayload::Error {
                            code: ErrorCode::BackendRejected,
                            message: "clipboard file was not accepted".into(),
                        },
                    },
                }
            }
            _ => ServerPayload::Error {
                code: ErrorCode::BackendRejected,
                message: "could not read the Wayland clipboard".into(),
            },
        },
        Method::ContactsList => ServerPayload::Contacts {
            contacts: snapshot(state).contacts,
        },
        Method::ContactsSync { device_id } => {
            let peer_id = device_id
                .as_str()
                .strip_prefix("native:")
                .unwrap_or_default();
            match native_backend().map(|native| native.request_contacts_sync(peer_id)) {
                Some(true) => ServerPayload::NativeAccepted,
                Some(false) => ServerPayload::Error {
                    code: ErrorCode::DeviceDisconnected,
                    message: "contacts sync was not accepted".into(),
                },
                None => ServerPayload::Error {
                    code: ErrorCode::BackendUnavailable,
                    message: "native backend unavailable".into(),
                },
            }
        }
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
        Method::ShareCancel {
            device_id,
            transfer_id,
        } => {
            return handle_share_cancel(device_id, transfer_id, writer, state, events).await;
        }
        Method::CustomCommands => ServerPayload::CustomCommands {
            commands: crate::custom_commands::entries(),
        },
        Method::CustomRun { name } => {
            let result = crate::custom_commands::run(&name).await;
            ServerPayload::CustomResult { result }
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
    if let ServerPayload::Subscribed {
        devices,
        notifications,
        media_sessions,
        calls,
        messaging_accounts,
        conversations,
        typing_states,
        read_states,
    } = response
    {
        let full = ServerMessage::new(ServerPayload::Subscribed {
            devices: devices.clone(),
            notifications: notifications.clone(),
            media_sessions: media_sessions.clone(),
            calls: calls.clone(),
            messaging_accounts: messaging_accounts.clone(),
            conversations: conversations.clone(),
            typing_states: typing_states.clone(),
            read_states: read_states.clone(),
        });
        if serde_json::to_vec(&full).map_or(true, |encoded| {
            encoded.len() + 1 > handover_ipc::MAX_LINE_BYTES
        }) {
            write_state_chunks(
                writer,
                SnapshotChunkKind::Subscribed,
                devices,
                notifications,
                media_sessions,
                calls,
                messaging_accounts,
                conversations,
                typing_states,
                read_states,
            )
            .await?;
        } else {
            write_json_line(writer, &full).await?;
        }
    } else {
        write_json_line(writer, &ServerMessage::new(response)).await?;
    }
    Ok(true)
}
