//! Versioned newline-delimited JSON protocol shared by `handoverd` and clients.

use std::env;
use std::path::PathBuf;

use handover_core::{
    Conversation, ConversationId, Device, DeviceEvent, DeviceId, MediaCommand, MediaEvent,
    MediaSession, MediaSessionId, Message, MessageId, MessageStatusUpdate, MessagingAccount,
    MessagingAccountId, MessagingEvent, Notification, NotificationEvent, NotificationId, ReadState,
    ReceivedShare, ShareResult, TypingState,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

pub const PROTOCOL_VERSION: u32 = 1;
/// Maximum decoded IPC line: 1 MiB. Proven floor: a real 86-thread
/// conversation list serializes to ~71 KiB, and full message windows
/// with bodies exceed the old 64 KiB bound deterministically, which made
/// subscribe/snapshot/history undecodable for real libraries. The daemon
/// already writes unbounded lines; this aligns readers. Per-connection
/// buffer cost is bounded by this cap.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;
const SOCKET_DIRECTORY: &str = "handover";
const SOCKET_NAME: &str = "handoverd.sock";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub protocol: u32,
    #[serde(flatten)]
    pub method: Method,
}

impl Request {
    pub fn new(method: Method) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            method,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "method")]
pub enum Method {
    #[serde(rename = "hello")]
    Hello,
    #[serde(rename = "devices.list")]
    DevicesList,
    #[serde(rename = "native.peers")]
    NativePeers,
    #[serde(rename = "native.pending")]
    NativePending,
    #[serde(rename = "native.pair")]
    NativePair { id: String, code: String },
    #[serde(rename = "native.unpair")]
    NativeUnpair { id: String },
    #[serde(rename = "notifications.list")]
    NotificationsList,
    #[serde(rename = "media.list")]
    MediaList,
    #[serde(rename = "subscribe")]
    Subscribe {
        #[serde(default)]
        shares: bool,
        #[serde(default)]
        media: bool,
        #[serde(default)]
        messages: bool,
    },
    #[serde(rename = "notification.dismiss")]
    NotificationDismiss { notification_id: NotificationId },
    #[serde(rename = "notification.action")]
    NotificationInvokeAction {
        notification_id: NotificationId,
        action_id: String,
    },
    #[serde(rename = "notification.reply")]
    NotificationReply {
        notification_id: NotificationId,
        text: String,
    },
    #[serde(rename = "share.url")]
    ShareUrl { device_id: DeviceId, url: String },
    #[serde(rename = "share.file")]
    ShareFile {
        device_id: DeviceId,
        file_url: String,
    },
    #[serde(rename = "media.control")]
    MediaControl {
        #[serde(flatten)]
        command: MediaCommand,
    },
    #[serde(rename = "messages.accounts")]
    MessagesAccounts,
    #[serde(rename = "messages.conversations")]
    MessagesConversations { account_id: MessagingAccountId },
    #[serde(rename = "messages.history")]
    MessagesHistory {
        conversation_id: ConversationId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cursor: Option<String>,
    },
    #[serde(rename = "messages.typing_states")]
    MessagesTypingStates,
    #[serde(rename = "messages.read_states")]
    MessagesReadStates,
    #[serde(rename = "messages.send")]
    MessagesSend {
        conversation_id: ConversationId,
        text: String,
    },
    #[serde(rename = "messages.send_file")]
    MessagesSendFile {
        conversation_id: ConversationId,
        file_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
    },
    #[serde(rename = "messages.react")]
    MessagesReact {
        message_id: MessageId,
        emoji: String,
    },
    #[serde(rename = "messages.unreact")]
    MessagesUnreact {
        message_id: MessageId,
        emoji: String,
    },
    #[serde(rename = "messages.read")]
    MessagesRead {
        conversation_id: ConversationId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<MessageId>,
    },
    #[serde(rename = "messages.typing")]
    MessagesTyping { conversation_id: ConversationId },
    #[serde(rename = "messages.delete")]
    MessagesDelete { message_id: MessageId },
    #[serde(rename = "messages.open")]
    MessagesOpen {
        account_id: MessagingAccountId,
        addresses: Vec<String>,
    },
    #[serde(rename = "messages.login")]
    MessagesLogin {
        account_id: MessagingAccountId,
        bundle_b64: String,
    },
    #[serde(rename = "messages.logout")]
    MessagesLogout { account_id: MessagingAccountId },
    #[serde(rename = "messages.sync")]
    MessagesSync { account_id: MessagingAccountId },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServerMessage {
    pub protocol: u32,
    #[serde(flatten)]
    pub payload: ServerPayload,
}

impl ServerMessage {
    pub fn new(payload: ServerPayload) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            payload,
        }
    }

    pub fn from_device_event(event: DeviceEvent) -> Self {
        let payload = match event {
            DeviceEvent::Added(device) => ServerPayload::DeviceAdded { device },
            DeviceEvent::Updated(device) => ServerPayload::DeviceUpdated { device },
            DeviceEvent::Removed(device_id) => ServerPayload::DeviceRemoved { device_id },
        };
        Self::new(payload)
    }

    pub fn from_notification_event(event: NotificationEvent) -> Self {
        let payload = match event {
            NotificationEvent::Added(notification) => {
                ServerPayload::NotificationAdded { notification }
            }
            NotificationEvent::Updated(notification) => {
                ServerPayload::NotificationUpdated { notification }
            }
            NotificationEvent::Removed(notification_id) => {
                ServerPayload::NotificationRemoved { notification_id }
            }
        };
        Self::new(payload)
    }

    pub fn from_media_event(event: MediaEvent) -> Self {
        let payload = match event {
            MediaEvent::Added(media_session) => ServerPayload::MediaAdded { media_session },
            MediaEvent::Updated(media_session) => ServerPayload::MediaUpdated { media_session },
            MediaEvent::Removed(media_session_id) => {
                ServerPayload::MediaRemoved { media_session_id }
            }
        };
        Self::new(payload)
    }

    pub fn from_messaging_event(event: MessagingEvent) -> Self {
        let payload = match event {
            MessagingEvent::Account(event) => match event {
                handover_core::MessagingAccountEvent::Added(account) => {
                    ServerPayload::AccountAdded { account }
                }
                handover_core::MessagingAccountEvent::Updated(account) => {
                    ServerPayload::AccountUpdated { account }
                }
                handover_core::MessagingAccountEvent::Removed(account_id) => {
                    ServerPayload::AccountRemoved { account_id }
                }
            },
            MessagingEvent::Conversation(event) => match event {
                handover_core::ConversationEvent::Added(conversation) => {
                    ServerPayload::ConversationAdded { conversation }
                }
                handover_core::ConversationEvent::Updated(conversation) => {
                    ServerPayload::ConversationUpdated { conversation }
                }
                handover_core::ConversationEvent::Removed(conversation_id) => {
                    ServerPayload::ConversationRemoved { conversation_id }
                }
            },
            MessagingEvent::Message(event) => match event {
                handover_core::MessageEvent::Added(message) => {
                    ServerPayload::MessageAdded { message }
                }
                handover_core::MessageEvent::Updated(message) => {
                    ServerPayload::MessageUpdated { message }
                }
                handover_core::MessageEvent::Removed(message_id) => {
                    ServerPayload::MessageRemoved { message_id }
                }
            },
            MessagingEvent::Status(update) => ServerPayload::MessageStatus { update },
            MessagingEvent::Typing(state) => ServerPayload::Typing { state },
            MessagingEvent::Read(state) => ServerPayload::ReadState { state },
            MessagingEvent::Pairing(prompt) => ServerPayload::Pairing {
                account_id: prompt.account_id,
                prompt: prompt.prompt,
            },
        };
        Self::new(payload)
    }

    pub fn protocol_error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::new(ServerPayload::Error {
            code,
            message: message.into(),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerPayload {
    Hello {
        supported_protocols: Vec<u32>,
    },
    Devices {
        devices: Vec<Device>,
    },
    NativePeers {
        peers: Vec<NativePeer>,
    },
    NativePending {
        pending: Vec<NativePendingPeer>,
    },
    NativeAccepted,
    Notifications {
        notifications: Vec<Notification>,
    },
    Media {
        media_sessions: Vec<MediaSession>,
    },
    Subscribed {
        devices: Vec<Device>,
        #[serde(default)]
        notifications: Vec<Notification>,
        #[serde(default)]
        media_sessions: Vec<MediaSession>,
        #[serde(default)]
        messaging_accounts: Vec<MessagingAccount>,
        #[serde(default)]
        conversations: Vec<Conversation>,
        #[serde(default)]
        typing_states: Vec<TypingState>,
        #[serde(default)]
        read_states: Vec<ReadState>,
    },
    Snapshot {
        devices: Vec<Device>,
        #[serde(default)]
        notifications: Vec<Notification>,
        #[serde(default)]
        media_sessions: Vec<MediaSession>,
        #[serde(default)]
        messaging_accounts: Vec<MessagingAccount>,
        #[serde(default)]
        conversations: Vec<Conversation>,
        #[serde(default)]
        typing_states: Vec<TypingState>,
        #[serde(default)]
        read_states: Vec<ReadState>,
    },
    DeviceAdded {
        device: Device,
    },
    DeviceUpdated {
        device: Device,
    },
    DeviceRemoved {
        device_id: DeviceId,
    },
    NotificationAdded {
        notification: Notification,
    },
    NotificationUpdated {
        notification: Notification,
    },
    NotificationRemoved {
        notification_id: NotificationId,
    },
    MediaAdded {
        media_session: MediaSession,
    },
    MediaUpdated {
        media_session: MediaSession,
    },
    MediaRemoved {
        media_session_id: MediaSessionId,
    },
    ShareReceived {
        share: ReceivedShare,
    },
    ShareResult {
        result: ShareResult,
    },
    ShareAccepted {
        device_id: DeviceId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transfer_id: Option<String>,
    },
    MediaAccepted {
        id: MediaSessionId,
    },
    Accounts {
        accounts: Vec<MessagingAccount>,
    },
    Conversations {
        conversations: Vec<Conversation>,
    },
    History {
        conversation_id: ConversationId,
        messages: Vec<Message>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cursor_next: Option<String>,
    },
    TypingStates {
        states: Vec<TypingState>,
    },
    ReadStates {
        states: Vec<ReadState>,
    },
    AccountAdded {
        account: MessagingAccount,
    },
    AccountUpdated {
        account: MessagingAccount,
    },
    AccountRemoved {
        account_id: MessagingAccountId,
    },
    ConversationAdded {
        conversation: Conversation,
    },
    ConversationUpdated {
        conversation: Conversation,
    },
    ConversationRemoved {
        conversation_id: ConversationId,
    },
    MessageAdded {
        message: Message,
    },
    MessageUpdated {
        message: Message,
    },
    MessageRemoved {
        message_id: MessageId,
    },
    MessageStatus {
        update: MessageStatusUpdate,
    },
    Typing {
        state: TypingState,
    },
    ReadState {
        state: ReadState,
    },
    Pairing {
        account_id: MessagingAccountId,
        prompt: String,
    },
    MessageAccepted {
        request_id: String,
    },
    ConversationAccepted {
        conversation_id: ConversationId,
    },
    AccountAccepted {
        account_id: MessagingAccountId,
    },
    CommandCompleted {
        notification_id: NotificationId,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativePeer {
    pub id: String,
    pub name: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativePendingPeer {
    pub id: String,
    pub name: String,
    pub code: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    MalformedRequest,
    UnsupportedProtocol,
    NotificationNotFound,
    InvalidNotificationCommand,
    BackendUnavailable,
    BackendRejected,
    UnknownDevice,
    DeviceDisconnected,
    DeviceNotPaired,
    UnsupportedCapability,
    InvalidResource,
    ResourceNotFound,
    MediaSessionNotFound,
    InvalidMediaCommand,
    MessagingUnavailable,
    UnknownMessagingAccount,
    UnknownConversation,
    UnknownMessage,
    UnsupportedMessagingCapability,
    InvalidMessagingCommand,
    HistoryUnavailable,
    CredentialRejected,
}

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("XDG_RUNTIME_DIR is not set")]
    MissingRuntimeDirectory,
    #[error("XDG_RUNTIME_DIR must be an absolute path: {0}")]
    RelativeRuntimeDirectory(PathBuf),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("IPC message exceeded {MAX_LINE_BYTES} bytes")]
    LineTooLong,
    #[error("daemon closed the connection")]
    ConnectionClosed,
    #[error("unexpected daemon response: {0}")]
    UnexpectedResponse(String),
    #[error("daemon rejected the request ({code:?}): {message}")]
    Server { code: ErrorCode, message: String },
}

pub fn runtime_directory() -> Result<PathBuf, IpcError> {
    let directory = env::var_os("XDG_RUNTIME_DIR").ok_or(IpcError::MissingRuntimeDirectory)?;
    let directory = PathBuf::from(directory);
    if !directory.is_absolute() {
        return Err(IpcError::RelativeRuntimeDirectory(directory));
    }
    Ok(directory.join(SOCKET_DIRECTORY))
}

pub fn socket_path() -> Result<PathBuf, IpcError> {
    Ok(runtime_directory()?.join(SOCKET_NAME))
}

pub async fn read_json_line<R, T>(reader: &mut R) -> Result<Option<T>, IpcError>
where
    R: AsyncBufRead + Unpin,
    T: DeserializeOwned,
{
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            break;
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |position| position + 1);
        let content_length = newline.unwrap_or(available.len());
        if line.len() + content_length > MAX_LINE_BYTES {
            return Err(IpcError::LineTooLong);
        }
        line.extend_from_slice(&available[..content_length]);
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }

    Ok(Some(serde_json::from_slice(&line)?))
}

pub async fn write_json_line<W, T>(writer: &mut W, value: &T) -> Result<(), IpcError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let encoded = serde_json::to_vec(value)?;
    writer.write_all(&encoded).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

pub struct Client {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl Client {
    pub async fn connect() -> Result<Self, IpcError> {
        Self::connect_to(socket_path()?).await
    }

    pub async fn connect_to(path: PathBuf) -> Result<Self, IpcError> {
        let stream = UnixStream::connect(path).await?;
        let (reader, writer) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(reader),
            writer,
        })
    }

    pub async fn hello(&mut self) -> Result<Vec<u32>, IpcError> {
        self.send(Method::Hello).await?;
        match self.receive().await?.payload {
            ServerPayload::Hello {
                supported_protocols,
            } => Ok(supported_protocols),
            payload => Err(unexpected(payload)),
        }
    }

    pub async fn devices(&mut self) -> Result<Vec<Device>, IpcError> {
        self.send(Method::DevicesList).await?;
        match self.receive().await?.payload {
            ServerPayload::Devices { devices } => Ok(devices),
            payload => Err(unexpected(payload)),
        }
    }

    pub async fn native_peers(&mut self) -> Result<Vec<NativePeer>, IpcError> {
        self.send(Method::NativePeers).await?;
        match self.receive().await?.payload {
            ServerPayload::NativePeers { peers } => Ok(peers),
            payload => Err(unexpected(payload)),
        }
    }

    pub async fn native_pending(&mut self) -> Result<Vec<NativePendingPeer>, IpcError> {
        self.send(Method::NativePending).await?;
        match self.receive().await?.payload {
            ServerPayload::NativePending { pending } => Ok(pending),
            payload => Err(unexpected(payload)),
        }
    }

    pub async fn native_pair(&mut self, id: String, code: String) -> Result<(), IpcError> {
        self.send(Method::NativePair { id, code }).await?;
        match self.receive().await?.payload {
            ServerPayload::NativeAccepted => Ok(()),
            payload => Err(unexpected(payload)),
        }
    }

    pub async fn native_unpair(&mut self, id: String) -> Result<(), IpcError> {
        self.send(Method::NativeUnpair { id }).await?;
        match self.receive().await?.payload {
            ServerPayload::NativeAccepted => Ok(()),
            payload => Err(unexpected(payload)),
        }
    }

    pub async fn notifications(&mut self) -> Result<Vec<Notification>, IpcError> {
        self.send(Method::NotificationsList).await?;
        match self.receive().await?.payload {
            ServerPayload::Notifications { notifications } => Ok(notifications),
            payload => Err(unexpected(payload)),
        }
    }

    pub async fn media_sessions(&mut self) -> Result<Vec<MediaSession>, IpcError> {
        self.send(Method::MediaList).await?;
        match self.receive().await?.payload {
            ServerPayload::Media { media_sessions } => Ok(media_sessions),
            payload => Err(unexpected(payload)),
        }
    }

    pub async fn media_command(&mut self, command: MediaCommand) -> Result<(), IpcError> {
        let id = command.id().clone();
        self.send(Method::MediaControl { command }).await?;
        match self.receive().await?.payload {
            ServerPayload::MediaAccepted { id: accepted } if accepted == id => Ok(()),
            payload => Err(unexpected(payload)),
        }
    }

    pub async fn dismiss_notification(
        &mut self,
        notification_id: NotificationId,
    ) -> Result<(), IpcError> {
        self.send(Method::NotificationDismiss {
            notification_id: notification_id.clone(),
        })
        .await?;
        self.expect_command_completed(notification_id).await
    }

    pub async fn invoke_notification_action(
        &mut self,
        notification_id: NotificationId,
        action_id: String,
    ) -> Result<(), IpcError> {
        self.send(Method::NotificationInvokeAction {
            notification_id: notification_id.clone(),
            action_id,
        })
        .await?;
        self.expect_command_completed(notification_id).await
    }

    pub async fn reply_to_notification(
        &mut self,
        notification_id: NotificationId,
        text: String,
    ) -> Result<(), IpcError> {
        self.send(Method::NotificationReply {
            notification_id: notification_id.clone(),
            text,
        })
        .await?;
        self.expect_command_completed(notification_id).await
    }

    pub async fn send_url(&mut self, device_id: DeviceId, url: String) -> Result<(), IpcError> {
        self.send_url_tracked(device_id, url).await.map(|_| ())
    }

    pub async fn send_url_tracked(
        &mut self,
        device_id: DeviceId,
        url: String,
    ) -> Result<Option<String>, IpcError> {
        self.send(Method::ShareUrl {
            device_id: device_id.clone(),
            url,
        })
        .await?;
        self.expect_share_accepted(device_id).await
    }

    pub async fn send_file_url(
        &mut self,
        device_id: DeviceId,
        file_url: String,
    ) -> Result<(), IpcError> {
        self.send_file_url_tracked(device_id, file_url)
            .await
            .map(|_| ())
    }

    pub async fn send_file_url_tracked(
        &mut self,
        device_id: DeviceId,
        file_url: String,
    ) -> Result<Option<String>, IpcError> {
        self.send(Method::ShareFile {
            device_id: device_id.clone(),
            file_url,
        })
        .await?;
        self.expect_share_accepted(device_id).await
    }

    async fn expect_share_accepted(
        &mut self,
        device_id: DeviceId,
    ) -> Result<Option<String>, IpcError> {
        match self.receive().await?.payload {
            ServerPayload::ShareAccepted {
                device_id: accepted,
                transfer_id,
            } if accepted == device_id => Ok(transfer_id),
            payload => Err(unexpected(payload)),
        }
    }

    async fn expect_command_completed(
        &mut self,
        notification_id: NotificationId,
    ) -> Result<(), IpcError> {
        match self.receive().await?.payload {
            ServerPayload::CommandCompleted {
                notification_id: completed,
            } if completed == notification_id => Ok(()),
            payload => Err(unexpected(payload)),
        }
    }

    pub async fn subscribe(mut self) -> Result<Subscription, IpcError> {
        self.send(Method::Subscribe {
            shares: true,
            media: true,
            messages: true,
        })
        .await?;
        match self.receive().await?.payload {
            ServerPayload::Subscribed {
                devices,
                notifications,
                media_sessions,
                ..
            } => Ok(Subscription {
                devices,
                notifications,
                media_sessions,
                reader: self.reader,
                _writer: self.writer,
            }),
            payload => Err(unexpected(payload)),
        }
    }

    pub async fn messaging_accounts(&mut self) -> Result<Vec<MessagingAccount>, IpcError> {
        self.send(Method::MessagesAccounts).await?;
        match self.receive().await?.payload {
            ServerPayload::Accounts { accounts } => Ok(accounts),
            payload => Err(unexpected(payload)),
        }
    }

    pub async fn messaging_conversations(
        &mut self,
        account_id: MessagingAccountId,
    ) -> Result<Vec<Conversation>, IpcError> {
        self.send(Method::MessagesConversations { account_id })
            .await?;
        match self.receive().await?.payload {
            ServerPayload::Conversations { conversations } => Ok(conversations),
            payload => Err(unexpected(payload)),
        }
    }

    pub async fn messaging_history(
        &mut self,
        conversation_id: ConversationId,
        limit: Option<u32>,
        cursor: Option<String>,
    ) -> Result<(Vec<Message>, Option<String>), IpcError> {
        self.send(Method::MessagesHistory {
            conversation_id,
            limit,
            cursor,
        })
        .await?;
        match self.receive().await?.payload {
            ServerPayload::History {
                messages,
                cursor_next,
                ..
            } => Ok((messages, cursor_next)),
            payload => Err(unexpected(payload)),
        }
    }

    async fn expect_message_accepted(&mut self) -> Result<String, IpcError> {
        match self.receive().await?.payload {
            ServerPayload::MessageAccepted { request_id } => Ok(request_id),
            payload => Err(unexpected(payload)),
        }
    }

    async fn expect_conversation_accepted(
        &mut self,
        conversation_id: ConversationId,
    ) -> Result<(), IpcError> {
        match self.receive().await?.payload {
            ServerPayload::ConversationAccepted {
                conversation_id: accepted,
            } if accepted == conversation_id => Ok(()),
            payload => Err(unexpected(payload)),
        }
    }

    async fn expect_account_accepted(
        &mut self,
        account_id: MessagingAccountId,
    ) -> Result<(), IpcError> {
        match self.receive().await?.payload {
            ServerPayload::AccountAccepted {
                account_id: accepted,
            } if accepted == account_id => Ok(()),
            payload => Err(unexpected(payload)),
        }
    }

    pub async fn send_message_text(
        &mut self,
        conversation_id: ConversationId,
        text: String,
    ) -> Result<String, IpcError> {
        self.send(Method::MessagesSend {
            conversation_id,
            text,
        })
        .await?;
        self.expect_message_accepted().await
    }

    pub async fn send_message_file(
        &mut self,
        conversation_id: ConversationId,
        file_url: String,
        caption: Option<String>,
    ) -> Result<String, IpcError> {
        self.send(Method::MessagesSendFile {
            conversation_id,
            file_url,
            caption,
        })
        .await?;
        self.expect_message_accepted().await
    }

    pub async fn react_to_message(
        &mut self,
        message_id: MessageId,
        emoji: String,
    ) -> Result<String, IpcError> {
        self.send(Method::MessagesReact { message_id, emoji })
            .await?;
        self.expect_message_accepted().await
    }

    pub async fn unreact_to_message(
        &mut self,
        message_id: MessageId,
        emoji: String,
    ) -> Result<String, IpcError> {
        self.send(Method::MessagesUnreact { message_id, emoji })
            .await?;
        self.expect_message_accepted().await
    }

    pub async fn mark_conversation_read(
        &mut self,
        conversation_id: ConversationId,
        message_id: Option<MessageId>,
    ) -> Result<(), IpcError> {
        self.send(Method::MessagesRead {
            conversation_id: conversation_id.clone(),
            message_id,
        })
        .await?;
        self.expect_conversation_accepted(conversation_id).await
    }

    pub async fn start_typing(&mut self, conversation_id: ConversationId) -> Result<(), IpcError> {
        self.send(Method::MessagesTyping {
            conversation_id: conversation_id.clone(),
        })
        .await?;
        self.expect_conversation_accepted(conversation_id).await
    }

    pub async fn delete_message(&mut self, message_id: MessageId) -> Result<String, IpcError> {
        self.send(Method::MessagesDelete { message_id }).await?;
        self.expect_message_accepted().await
    }

    pub async fn open_conversation(
        &mut self,
        account_id: MessagingAccountId,
        addresses: Vec<String>,
    ) -> Result<String, IpcError> {
        self.send(Method::MessagesOpen {
            account_id,
            addresses,
        })
        .await?;
        self.expect_message_accepted().await
    }

    pub async fn messaging_login(
        &mut self,
        account_id: MessagingAccountId,
        bundle_b64: String,
    ) -> Result<(), IpcError> {
        self.send(Method::MessagesLogin {
            account_id: account_id.clone(),
            bundle_b64,
        })
        .await?;
        self.expect_account_accepted(account_id).await
    }

    pub async fn messaging_logout(
        &mut self,
        account_id: MessagingAccountId,
    ) -> Result<(), IpcError> {
        self.send(Method::MessagesLogout {
            account_id: account_id.clone(),
        })
        .await?;
        self.expect_account_accepted(account_id).await
    }

    pub async fn messaging_sync(&mut self, account_id: MessagingAccountId) -> Result<(), IpcError> {
        self.send(Method::MessagesSync {
            account_id: account_id.clone(),
        })
        .await?;
        self.expect_account_accepted(account_id).await
    }

    async fn send(&mut self, method: Method) -> Result<(), IpcError> {
        write_json_line(&mut self.writer, &Request::new(method)).await
    }

    async fn receive(&mut self) -> Result<ServerMessage, IpcError> {
        receive_message(&mut self.reader).await
    }
}

pub struct Subscription {
    pub devices: Vec<Device>,
    pub notifications: Vec<Notification>,
    pub media_sessions: Vec<MediaSession>,
    reader: BufReader<OwnedReadHalf>,
    _writer: OwnedWriteHalf,
}

impl Subscription {
    pub async fn next_message(&mut self) -> Result<ServerMessage, IpcError> {
        receive_message(&mut self.reader).await
    }
}

async fn receive_message(reader: &mut BufReader<OwnedReadHalf>) -> Result<ServerMessage, IpcError> {
    let message: ServerMessage = read_json_line(reader)
        .await?
        .ok_or(IpcError::ConnectionClosed)?;
    if message.protocol != PROTOCOL_VERSION {
        return Err(IpcError::UnexpectedResponse(format!(
            "protocol {}",
            message.protocol
        )));
    }
    if let ServerPayload::Error { code, message } = message.payload {
        return Err(IpcError::Server { code, message });
    }
    Ok(message)
}

fn unexpected(payload: ServerPayload) -> IpcError {
    IpcError::UnexpectedResponse(format!("{payload:?}"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use handover_core::{BatteryState, Capability};

    use super::*;

    fn device() -> Device {
        Device {
            id: DeviceId::new("phone-123"),
            name: "Phone".into(),
            connected: true,
            paired: true,
            battery: Some(BatteryState::new(76, false).expect("valid battery")),
            capabilities: BTreeSet::from([Capability::Battery]),
        }
    }

    #[test]
    fn request_round_trips_as_expected_json() {
        let request = Request::new(Method::DevicesList);
        let json = serde_json::to_string(&request).expect("request serializes");

        assert_eq!(json, r#"{"protocol":1,"method":"devices.list"}"#);
        assert_eq!(
            serde_json::from_str::<Request>(&json).expect("request deserializes"),
            request
        );
    }

    #[test]
    fn snapshot_round_trips_with_domain_device() {
        let message = ServerMessage::new(ServerPayload::Devices {
            devices: vec![device()],
        });
        let json = serde_json::to_string(&message).expect("response serializes");
        let decoded: ServerMessage = serde_json::from_str(&json).expect("response deserializes");

        assert_eq!(decoded, message);
        assert!(json.contains(r#""type":"devices""#));
        assert!(json.contains(r#""percentage":76"#));
    }

    #[test]
    fn device_event_maps_without_backend_details() {
        let message = ServerMessage::from_device_event(DeviceEvent::Added(device()));

        assert!(matches!(message.payload, ServerPayload::DeviceAdded { .. }));
        assert!(
            !serde_json::to_string(&message)
                .expect("event serializes")
                .contains("kdeconnect")
        );
    }

    #[test]
    fn protocol_version_is_required() {
        let error = serde_json::from_str::<Request>(r#"{"method":"hello"}"#)
            .expect_err("protocol is required");

        assert!(error.to_string().contains("protocol"));
    }

    fn notification() -> Notification {
        Notification {
            id: NotificationId::new(DeviceId::new("phone-123"), "42"),
            app_name: "Messages".into(),
            title: "Alex".into(),
            body: "Hello".into(),
            icon_path: None,
            clearable: true,
            actions: vec![],
            reply_supported: true,
        }
    }

    fn media_session() -> MediaSession {
        MediaSession {
            id: MediaSessionId::new(DeviceId::new("phone-123"), "player-1"),
            application: "Music".into(),
            title: Some("Song".into()),
            artist: Some("Artist".into()),
            album: None,
            playback: handover_core::PlaybackState::Playing,
            position_ms: Some(1_000),
            duration_ms: Some(120_000),
            volume_percent: Some(80),
            controls: BTreeSet::from([handover_core::MediaControl::Play]),
        }
    }

    #[test]
    fn notification_request_uses_versioned_method_names() {
        let request = Request::new(Method::NotificationReply {
            notification_id: notification().id,
            text: "Thanks".into(),
        });
        let json = serde_json::to_string(&request).expect("request serializes");

        assert_eq!(
            json,
            r#"{"protocol":1,"method":"notification.reply","notification_id":{"device_id":"phone-123","local_id":"42"},"text":"Thanks"}"#
        );
    }

    #[test]
    fn notification_event_maps_to_backend_independent_wire_event() {
        let message =
            ServerMessage::from_notification_event(NotificationEvent::Added(notification()));
        let json = serde_json::to_string(&message).expect("event serializes");

        assert!(matches!(
            message.payload,
            ServerPayload::NotificationAdded { .. }
        ));
        assert!(json.contains(r#""type":"notification_added""#));
        assert!(!json.contains("kdeconnect"));
    }

    #[test]
    fn subscription_and_snapshot_default_missing_notifications() {
        let subscribed = serde_json::from_str::<ServerMessage>(
            r#"{"protocol":1,"type":"subscribed","devices":[]}"#,
        )
        .expect("legacy subscription decodes");
        let snapshot = serde_json::from_str::<ServerMessage>(
            r#"{"protocol":1,"type":"snapshot","devices":[]}"#,
        )
        .expect("legacy snapshot decodes");

        assert!(matches!(
            subscribed.payload,
            ServerPayload::Subscribed { notifications, .. } if notifications.is_empty()
        ));
        assert!(matches!(
            snapshot.payload,
            ServerPayload::Snapshot { notifications, .. } if notifications.is_empty()
        ));
    }

    #[test]
    fn notification_snapshot_round_trips() {
        let message = ServerMessage::new(ServerPayload::Snapshot {
            devices: vec![device()],
            notifications: vec![notification()],
            media_sessions: vec![],
            messaging_accounts: vec![],
            conversations: vec![],
            typing_states: vec![],
            read_states: vec![],
        });
        let decoded: ServerMessage =
            serde_json::from_str(&serde_json::to_string(&message).expect("serializes"))
                .expect("deserializes");

        assert_eq!(decoded, message);
    }

    #[test]
    fn media_request_uses_flattened_versioned_command() {
        let request = Request::new(Method::MediaControl {
            command: MediaCommand::Play {
                id: media_session().id,
            },
        });
        let json = serde_json::to_string(&request).expect("request serializes");

        assert_eq!(
            json,
            r#"{"protocol":1,"method":"media.control","action":"play","id":{"device_id":"phone-123","player_id":"player-1"}}"#
        );
        assert_eq!(
            serde_json::from_str::<Request>(&json).expect("request deserializes"),
            request
        );
    }

    #[test]
    fn media_request_rejects_unknown_action_and_missing_identity() {
        let unknown = r#"{"protocol":1,"method":"media.control","action":"explode","id":{"device_id":"phone","player_id":"Player"}}"#;
        let missing = r#"{"protocol":1,"method":"media.control","action":"play"}"#;
        assert!(serde_json::from_str::<Request>(unknown).is_err());
        assert!(serde_json::from_str::<Request>(missing).is_err());
    }

    #[test]
    fn media_snapshot_and_events_round_trip() {
        let session = media_session();
        let snapshot = ServerMessage::new(ServerPayload::Snapshot {
            devices: vec![device()],
            notifications: vec![],
            media_sessions: vec![session.clone()],
            messaging_accounts: vec![],
            conversations: vec![],
            typing_states: vec![],
            read_states: vec![],
        });
        let decoded: ServerMessage =
            serde_json::from_str(&serde_json::to_string(&snapshot).expect("snapshot serializes"))
                .expect("snapshot deserializes");
        assert_eq!(decoded, snapshot);

        let event = ServerMessage::from_media_event(MediaEvent::Updated(session));
        assert!(matches!(event.payload, ServerPayload::MediaUpdated { .. }));
    }

    #[test]
    fn share_requests_use_protocol_one_without_backend_details() {
        let request = Request::new(Method::ShareUrl {
            device_id: DeviceId::new("phone-a"),
            url: "https://example.com/path".into(),
        });
        let encoded = serde_json::to_string(&request).expect("serializes");
        assert_eq!(
            encoded,
            r#"{"protocol":1,"method":"share.url","device_id":"phone-a","url":"https://example.com/path"}"#
        );
        assert_eq!(
            serde_json::from_str::<Request>(&encoded).expect("deserializes"),
            request
        );

        let file = Request::new(Method::ShareFile {
            device_id: DeviceId::new("phone-a"),
            file_url: "file:///tmp/handover%20test.txt".into(),
        });
        assert_eq!(
            serde_json::from_str::<Request>(&serde_json::to_string(&file).expect("serializes"))
                .expect("deserializes"),
            file
        );
    }

    #[test]
    fn incoming_share_event_round_trips() {
        let message = ServerMessage::new(ServerPayload::ShareReceived {
            share: ReceivedShare {
                device_id: DeviceId::new("phone-a"),
                resource: handover_core::SharedResource::Url {
                    url: "https://example.com".into(),
                },
            },
        });
        let encoded = serde_json::to_string(&message).expect("serializes");
        assert!(encoded.contains(r#""type":"share_received""#));
        assert_eq!(
            serde_json::from_str::<ServerMessage>(&encoded).expect("deserializes"),
            message
        );
    }

    #[test]
    fn native_result_is_additive_and_kde_acceptance_has_no_transfer_id() {
        let result = ServerMessage::new(ServerPayload::ShareResult {
            result: handover_core::ShareResult {
                device_id: DeviceId::new("phone-a"),
                transfer_id: "0123456789abcdef0123456789abcdef".into(),
                status: handover_core::ShareStatus::Failed,
                reason: Some(handover_core::ShareFailure::TimedOut),
            },
        });
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(encoded.contains("\"type\":\"share_result\""));
        assert_eq!(
            serde_json::from_str::<ServerMessage>(&encoded).unwrap(),
            result
        );
        let kde = ServerMessage::new(ServerPayload::ShareAccepted {
            device_id: DeviceId::new("phone-a"),
            transfer_id: None,
        });
        assert!(!serde_json::to_string(&kde).unwrap().contains("transfer_id"));
    }

    #[test]
    fn messaging_methods_use_versioned_names() {
        let account = MessagingAccountId::new("gmessages:default");
        let conversation = ConversationId::new(account.clone(), "thread-1");
        let request = Request::new(Method::MessagesSend {
            conversation_id: conversation.clone(),
            text: "hello".into(),
        });
        let json = serde_json::to_string(&request).expect("serializes");
        assert_eq!(
            json,
            r#"{"protocol":1,"method":"messages.send","conversation_id":{"account_id":"gmessages:default","local_id":"thread-1"},"text":"hello"}"#
        );
        assert_eq!(
            serde_json::from_str::<Request>(&json).expect("deserializes"),
            request
        );

        let history = Request::new(Method::MessagesHistory {
            conversation_id: conversation,
            limit: Some(25),
            cursor: None,
        });
        let json = serde_json::to_string(&history).expect("serializes");
        assert!(json.contains(r#""method":"messages.history""#));
        assert!(json.contains(r#""limit":25"#));
        assert!(!json.contains("cursor"));
    }

    #[test]
    fn messaging_events_round_trip_without_backend_details() {
        let message = ServerMessage::from_messaging_event(MessagingEvent::Typing(TypingState {
            conversation_id: ConversationId::new(
                MessagingAccountId::new("gmessages:default"),
                "thread-1",
            ),
            participant_ids: vec!["peer".into()],
        }));
        let json = serde_json::to_string(&message).expect("serializes");
        assert!(json.contains(r#""type":"typing""#));
        assert!(!json.contains("kdeconnect"));
        assert!(!json.contains("bugle"));
        assert_eq!(
            serde_json::from_str::<ServerMessage>(&json).expect("deserializes"),
            message
        );
    }

    #[test]
    fn legacy_snapshot_decodes_without_messaging_fields() {
        let snapshot = serde_json::from_str::<ServerMessage>(
            r#"{"protocol":1,"type":"snapshot","devices":[]}"#,
        )
        .expect("legacy snapshot decodes");
        assert!(matches!(
            snapshot.payload,
            ServerPayload::Snapshot {
                messaging_accounts,
                conversations,
                ..
            } if messaging_accounts.is_empty() && conversations.is_empty()
        ));
    }

    #[test]
    fn share_subscription_is_opt_in_for_older_protocol_one_clients() {
        let legacy: Request = serde_json::from_str(r#"{"protocol":1,"method":"subscribe"}"#)
            .expect("old subscription still decodes");
        assert_eq!(
            legacy.method,
            Method::Subscribe {
                shares: false,
                media: false,
                messages: false,
            }
        );
        let current = Request::new(Method::Subscribe {
            shares: true,
            media: true,
            messages: true,
        });
        assert_eq!(
            serde_json::to_string(&current).expect("serializes"),
            r#"{"protocol":1,"method":"subscribe","shares":true,"media":true,"messages":true}"#
        );
    }

    #[tokio::test]
    async fn large_library_snapshot_lines_decode() {
        // Regression: a real 86-thread conversation list measures ~71 KiB
        // serialized, which the old 64 KiB bound rejected, breaking
        // subscribe/snapshot/history for real libraries.
        let bodies = "x".repeat(100 * 1024);
        let message = ServerMessage::new(ServerPayload::Notifications {
            notifications: vec![Notification {
                id: NotificationId::new(DeviceId::new("phone"), "1"),
                app_name: "Messages".into(),
                title: "Big".into(),
                body: bodies,
                icon_path: None,
                clearable: true,
                actions: vec![],
                reply_supported: true,
            }],
        });
        let mut encoded = serde_json::to_vec(&message).expect("serializes");
        assert!(encoded.len() > 64 * 1024);
        encoded.push(b'\n');
        let mut reader = BufReader::new(encoded.as_slice());
        let decoded: Option<ServerMessage> = read_json_line(&mut reader).await.expect("decodes");
        assert_eq!(decoded, Some(message));
    }
}
