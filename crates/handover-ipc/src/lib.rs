//! Versioned newline-delimited JSON protocol shared by `handoverd` and clients.

use std::env;
use std::path::PathBuf;

use handover_core::{
    Device, DeviceEvent, DeviceId, MediaCommand, MediaEvent, MediaSession, MediaSessionId,
    Notification, NotificationEvent, NotificationId, ReceivedShare,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_LINE_BYTES: usize = 64 * 1024;
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
    },
    Snapshot {
        devices: Vec<Device>,
        #[serde(default)]
        notifications: Vec<Notification>,
        #[serde(default)]
        media_sessions: Vec<MediaSession>,
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
    ShareAccepted {
        device_id: DeviceId,
    },
    MediaAccepted {
        id: MediaSessionId,
    },
    CommandCompleted {
        notification_id: NotificationId,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
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
        self.send(Method::ShareFile {
            device_id: device_id.clone(),
            file_url,
        })
        .await?;
        self.expect_share_accepted(device_id).await
    }

    async fn expect_share_accepted(&mut self, device_id: DeviceId) -> Result<(), IpcError> {
        match self.receive().await?.payload {
            ServerPayload::ShareAccepted {
                device_id: accepted,
            } if accepted == device_id => Ok(()),
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
        })
        .await?;
        match self.receive().await?.payload {
            ServerPayload::Subscribed {
                devices,
                notifications,
                media_sessions,
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
    fn share_subscription_is_opt_in_for_older_protocol_one_clients() {
        let legacy: Request = serde_json::from_str(r#"{"protocol":1,"method":"subscribe"}"#)
            .expect("old subscription still decodes");
        assert_eq!(
            legacy.method,
            Method::Subscribe {
                shares: false,
                media: false,
            }
        );
        let current = Request::new(Method::Subscribe {
            shares: true,
            media: true,
        });
        assert_eq!(
            serde_json::to_string(&current).expect("serializes"),
            r#"{"protocol":1,"method":"subscribe","shares":true,"media":true}"#
        );
    }
}
