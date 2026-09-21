//! Backend-independent domain types used across Handover.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

pub mod messaging;

pub use messaging::{
    Attachment, AttachmentKind, Conversation, ConversationEvent, ConversationId, ConversationKind,
    MAX_PAGE_LIMIT, MAX_TEXT_CHARS, Message, MessageEvent, MessageId, MessageStatus,
    MessageStatusUpdate, MessagingAccount, MessagingAccountEvent, MessagingAccountId,
    MessagingCapability, MessagingCommand, MessagingEvent, PairingPrompt, Participant, Reaction,
    ReadState, SendFailure, TransportKind, TypingState, ValidationError, sanitize_file_name,
    validate_account, validate_command, validate_conversation, validate_message,
};

/// A stable identifier assigned to a device by a backend.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct DeviceId(String);

impl DeviceId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Normalized state for a device known to Handover.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub connected: bool,
    pub paired: bool,
    pub battery: Option<BatteryState>,
    #[serde(default)]
    pub connectivity: Option<ConnectivityState>,
    pub capabilities: BTreeSet<Capability>,
}

/// Network state reported by the device. This describes the active transport,
/// not reachability of the Handover session itself.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConnectivityState {
    pub transport: ConnectivityTransport,
    pub validated: bool,
    pub metered: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectivityTransport {
    Wifi,
    Ethernet,
    Cellular,
    Bluetooth,
    Other,
    None,
}

/// The battery information most recently reported by a device.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct BatteryState {
    percentage: u8,
    pub charging: bool,
}

impl<'de> Deserialize<'de> for BatteryState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SerializedBattery {
            percentage: u8,
            charging: bool,
        }

        let battery = SerializedBattery::deserialize(deserializer)?;
        Self::new(battery.percentage, battery.charging).map_err(serde::de::Error::custom)
    }
}

impl BatteryState {
    pub fn new(percentage: u8, charging: bool) -> Result<Self, BatteryStateError> {
        if percentage > 100 {
            return Err(BatteryStateError::InvalidPercentage(percentage));
        }

        Ok(Self {
            percentage,
            charging,
        })
    }

    pub fn percentage(self) -> u8 {
        self.percentage
    }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum BatteryStateError {
    #[error("battery percentage must be between 0 and 100, got {0}")]
    InvalidPercentage(u8),
}

/// A normalized feature that a device can provide.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Battery,
    Connectivity,
    FileTransfer,
    Media,
    Notifications,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationAction {
    Previous,
    Next,
    Start,
    Stop,
    Fullscreen,
    PointerMove,
    PointerClick,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PresentationCommand {
    pub device_id: DeviceId,
    pub action: PresentationAction,
    #[serde(default)]
    pub delta_x: i32,
    #[serde(default)]
    pub delta_y: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeAction {
    Up,
    Down,
    ToggleMute,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VolumeCommand {
    pub device_id: DeviceId,
    pub action: VolumeAction,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteInputAction {
    Move,
    Click,
    Scroll,
    Type,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RemoteInputCommand {
    pub device_id: DeviceId,
    pub action: RemoteInputAction,
    pub delta_x: i32,
    pub delta_y: i32,
    pub button: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClipboardText {
    pub device_id: DeviceId,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClipboardFile {
    pub device_id: DeviceId,
    pub path: String,
    pub mime: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Contact {
    pub device_id: DeviceId,
    pub local_id: String,
    pub display_name: String,
    pub phones: Vec<String>,
    pub emails: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ContactsEvent {
    Synced {
        device_id: DeviceId,
        contacts: Vec<Contact>,
    },
    Removed(DeviceId),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallPhase {
    Unknown,
    Idle,
    Ringing,
    /// Dialing or an ongoing call; does not attest that the remote party answered.
    OffHook,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallAction {
    Place,
    Answer,
    Decline,
    Hangup,
}

impl CallAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Place => "place",
            Self::Answer => "answer",
            Self::Decline => "decline",
            Self::Hangup => "hangup",
        }
    }
}

/// Deliberately excludes service codes, extensions, pauses and URI syntax.
/// The phone must additionally reject locally recognized emergency numbers.
pub fn valid_call_address(address: &str) -> bool {
    let digits = address.strip_prefix('+').unwrap_or(address);
    !digits.is_empty() && digits.len() <= 15 && digits.bytes().all(|b| b.is_ascii_digit())
}

/// Read-only, host-wide audio observations. Not associated with a particular
/// phone and never confirmation of audibility, microphone routing or delivery.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CallAudioStatus {
    pub observed: bool,
    pub gateway_ready: bool,
    pub duplex_running: bool,
}

/// Current phone-call state attested by a device.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CallState {
    pub device_id: DeviceId,
    pub phase: CallPhase,
    #[serde(default)]
    pub controls: BTreeSet<CallAction>,
    /// Bumped by the backend on every phone-reported call state. Queued call
    /// commands carry it; a newer report proves re-observation and makes the
    /// command stale, so it can never act on a later call.
    #[serde(default)]
    pub generation: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum CallEvent {
    Updated(CallState),
    Removed(DeviceId),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallCommandFailure {
    PermissionDenied,
    StaleState,
    EmergencyNumber,
    InvalidAddress,
    WrongPhase,
    Unsupported,
    Rejected,
}

/// Android's verdict on attempting a queued call control. `accepted` means
/// the platform API accepted the operation, never that a call connected.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CallCommandResult {
    pub device_id: DeviceId,
    pub request_id: String,
    pub action: CallAction,
    pub accepted: bool,
    pub failure: Option<CallCommandFailure>,
}

/// One entry of the desktop custom-command allowlist. The program and its
/// arguments are fixed by the local user; remote callers supply only the name.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CustomCommandEntry {
    pub name: String,
    pub argv: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomCommandFailure {
    UnknownCommand,
    SpawnFailed,
    TimedOut,
}

/// The outcome of running one allowlisted command. `accepted` means the
/// listed program ran; `exit_code` is its wait status, never its output.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CustomCommandResult {
    pub name: String,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<CustomCommandFailure>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FilesystemEntry {
    pub name: String,
    pub directory: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FilesystemResult {
    pub device_id: DeviceId,
    pub request_id: String,
    pub path: String,
    pub entries: Vec<FilesystemEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceCommandAction {
    Ping,
    Ring,
    Lock,
    Clipboard,
    Notification,
    Media,
    Screensaver,
    KeepAwake,
    Tethering,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceCommandFailure {
    PermissionDenied,
    Unavailable,
    Rejected,
}

/// Android's verdict after handling a queued fixed device command. Success
/// means the platform API was invoked, not that the user observed its effect.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeviceCommandResult {
    pub device_id: DeviceId,
    pub request_id: String,
    pub action: DeviceCommandAction,
    pub accepted: bool,
    pub failure: Option<DeviceCommandFailure>,
}

/// A change to the set of devices or to a device's normalized state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DeviceEvent {
    Added(Device),
    Updated(Device),
    Removed(DeviceId),
}

/// An identifier scoped by the device that produced the notification.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct NotificationId {
    pub device_id: DeviceId,
    pub local_id: String,
}

impl NotificationId {
    pub fn new(device_id: DeviceId, local_id: impl Into<String>) -> Self {
        Self {
            device_id,
            local_id: local_id.into(),
        }
    }
}

impl fmt::Display for NotificationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.device_id, self.local_id)
    }
}

/// An opaque action identifier and its user-visible label.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
}

/// Current normalized state for a notification originating from a device.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Notification {
    pub id: NotificationId,
    pub app_name: String,
    pub title: String,
    pub body: String,
    pub icon_path: Option<String>,
    pub clearable: bool,
    pub actions: Vec<NotificationAction>,
    pub reply_supported: bool,
}

/// A change to the daemon's current set of remote notifications.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum NotificationEvent {
    Added(Notification),
    Updated(Notification),
    Removed(NotificationId),
}

/// A backend-independent operation requested for a current notification.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum NotificationCommand {
    Dismiss {
        notification_id: NotificationId,
    },
    InvokeAction {
        notification_id: NotificationId,
        action_id: String,
    },
    Reply {
        notification_id: NotificationId,
        text: String,
    },
}

impl NotificationCommand {
    pub fn notification_id(&self) -> &NotificationId {
        match self {
            Self::Dismiss { notification_id }
            | Self::InvokeAction {
                notification_id, ..
            }
            | Self::Reply {
                notification_id, ..
            } => notification_id,
        }
    }
}

/// A normalized device, notification, media, share, or messaging transition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum StateEvent {
    Device(DeviceEvent),
    Notification(NotificationEvent),
    Media(MediaEvent),
    Call(CallEvent),
    CallCommandResult(CallCommandResult),
    DeviceCommandResult(DeviceCommandResult),
    ShareReceived(ReceivedShare),
    ShareProgress(ShareProgress),
    ShareResult(ShareResult),
    Messaging(MessagingEvent),
    Presentation(PresentationCommand),
    Volume(VolumeCommand),
    Clipboard(ClipboardText),
    ClipboardFile(ClipboardFile),
    RemoteInput(RemoteInputCommand),
    Filesystem(FilesystemResult),
    Contacts(ContactsEvent),
}

/// A media player identity scoped to its source device.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct MediaSessionId {
    pub device_id: DeviceId,
    pub player_id: String,
}

impl MediaSessionId {
    pub fn new(device_id: DeviceId, player_id: impl Into<String>) -> Self {
        Self {
            device_id,
            player_id: player_id.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaControl {
    Play,
    Pause,
    PlayPause,
    Next,
    Previous,
    Seek,
    SetPosition,
}

/// Current state of one remote media player. Times are milliseconds.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MediaSession {
    pub id: MediaSessionId,
    pub application: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub playback: PlaybackState,
    pub position_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub volume_percent: Option<u8>,
    pub controls: BTreeSet<MediaControl>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum MediaEvent {
    Added(MediaSession),
    Updated(MediaSession),
    Removed(MediaSessionId),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MediaCommand {
    Play {
        id: MediaSessionId,
    },
    Pause {
        id: MediaSessionId,
    },
    PlayPause {
        id: MediaSessionId,
    },
    Next {
        id: MediaSessionId,
    },
    Previous {
        id: MediaSessionId,
    },
    Seek {
        id: MediaSessionId,
        offset_ms: i64,
    },
    SetPosition {
        id: MediaSessionId,
        position_ms: u64,
    },
}

impl MediaCommand {
    pub fn id(&self) -> &MediaSessionId {
        match self {
            Self::Play { id }
            | Self::Pause { id }
            | Self::PlayPause { id }
            | Self::Next { id }
            | Self::Previous { id } => id,
            Self::Seek { id, .. } | Self::SetPosition { id, .. } => id,
        }
    }

    pub fn control(&self) -> MediaControl {
        match self {
            Self::Play { .. } => MediaControl::Play,
            Self::Pause { .. } => MediaControl::Pause,
            Self::PlayPause { .. } => MediaControl::PlayPause,
            Self::Next { .. } => MediaControl::Next,
            Self::Previous { .. } => MediaControl::Previous,
            Self::Seek { .. } => MediaControl::Seek,
            Self::SetPosition { .. } => MediaControl::SetPosition,
        }
    }
}

/// A resource made available on Linux after a remote share.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SharedResource {
    File { path: String },
    Url { url: String },
}

/// A transient incoming share event, not a transfer-progress record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReceivedShare {
    pub device_id: DeviceId,
    pub resource: SharedResource,
}

/// A transient result for one native share. KDE Connect cannot confirm delivery.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShareResult {
    pub device_id: DeviceId,
    pub transfer_id: String,
    pub status: ShareStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<ShareFailure>,
}

/// Transient sender progress for one native file transfer. This reports bytes
/// written to the authenticated stream, not receiver storage or delivery.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShareProgress {
    pub device_id: DeviceId,
    pub transfer_id: String,
    pub bytes_sent: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareStatus {
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareFailure {
    InvalidResource,
    SizeLimit,
    Storage,
    Interrupted,
    Rejected,
    TimedOut,
    Disconnected,
    Transport,
}

#[cfg(test)]
mod messaging_tests;
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_address_accepts_one_to_fifteen_ascii_digits_with_optional_leading_plus() {
        for length in 1..=15 {
            let digits = "012345678901234"[..length].to_string();
            assert!(valid_call_address(&digits), "rejected {digits:?}");
            let international = format!("+{digits}");
            assert!(
                valid_call_address(&international),
                "rejected {international:?}"
            );
        }
    }

    #[test]
    fn call_address_rejects_empty_overlong_and_non_ascii_digits() {
        for address in [
            "",
            "+",
            "0123456789012345",
            "+0123456789012345",
            "١٢٣",
            "+１２３",
            "1٢3",
            "abc",
            "1a2",
            "++123",
            "12+3",
            "123+",
        ] {
            assert!(!valid_call_address(address), "accepted {address:?}");
        }
    }

    #[test]
    fn call_address_rejects_service_codes_uris_and_separators() {
        for address in [
            "*123#",
            "*123",
            "123#",
            "tel:123",
            "tel:+123",
            "sip:123@example.com",
            "123@example.com",
            "%2B123",
            " 123",
            "123 ",
            "1 23",
            "1-23",
            "(123)",
            "1.23",
            "1/23",
            "1,23",
            "1;23",
            "123x4",
            "123;ext=4",
            "123p4",
            "123w4",
            "1\t23",
            "123\n",
            "123\r",
            "123\0",
        ] {
            assert!(!valid_call_address(address), "accepted {address:?}");
        }
    }

    #[test]
    fn device_id_preserves_backend_identifier() {
        let id = DeviceId::new("phone-123");

        assert_eq!(id.as_str(), "phone-123");
        assert_eq!(id.to_string(), "phone-123");
    }

    #[test]
    fn battery_state_accepts_boundary_percentages() {
        let empty = BatteryState::new(0, false).expect("zero is a valid percentage");
        let full = BatteryState::new(100, true).expect("100 is a valid percentage");

        assert_eq!(empty.percentage(), 0);
        assert!(!empty.charging);
        assert_eq!(full.percentage(), 100);
        assert!(full.charging);
    }

    #[test]
    fn battery_state_rejects_percentage_over_100() {
        assert_eq!(
            BatteryState::new(101, false),
            Err(BatteryStateError::InvalidPercentage(101))
        );
    }

    #[test]
    fn battery_state_deserialization_preserves_invariant() {
        let error = serde_json::from_str::<BatteryState>(r#"{"percentage":101,"charging":false}"#)
            .expect_err("invalid battery must not deserialize");

        assert!(error.to_string().contains("between 0 and 100"));
    }

    #[test]
    fn device_event_carries_normalized_device_state() {
        let device = Device {
            id: DeviceId::new("phone-123"),
            name: "Phone".into(),
            connected: true,
            paired: true,
            battery: Some(BatteryState::new(82, false).expect("valid battery state")),
            connectivity: None,
            capabilities: BTreeSet::from([Capability::Battery]),
        };

        assert_eq!(
            DeviceEvent::Added(device.clone()),
            DeviceEvent::Added(device)
        );
    }

    #[test]
    fn notification_identity_is_scoped_to_its_device() {
        let first = NotificationId::new(DeviceId::new("phone-a"), "42");
        let second = NotificationId::new(DeviceId::new("phone-b"), "42");

        assert_ne!(first, second);
        assert_eq!(first.to_string(), "phone-a:42");
    }

    #[test]
    fn received_share_is_backend_independent() {
        let share = ReceivedShare {
            device_id: DeviceId::new("phone-a"),
            resource: SharedResource::File {
                path: "/tmp/handover test ✓.txt".into(),
            },
        };
        let encoded = serde_json::to_string(&share).expect("share serializes");
        assert_eq!(
            encoded,
            r#"{"device_id":"phone-a","resource":{"kind":"file","path":"/tmp/handover test ✓.txt"}}"#
        );
        assert_eq!(
            serde_json::from_str::<ReceivedShare>(&encoded).expect("share deserializes"),
            share
        );
    }

    #[test]
    fn media_identity_is_scoped_to_device_and_player() {
        let first = MediaSessionId::new(DeviceId::new("phone-a"), "Spotify");
        assert_ne!(
            first,
            MediaSessionId::new(DeviceId::new("phone-b"), "Spotify")
        );
        assert_ne!(
            first,
            MediaSessionId::new(DeviceId::new("phone-a"), "Browser")
        );
    }

    #[test]
    fn media_command_round_trips_without_backend_details() {
        let command = MediaCommand::Seek {
            id: MediaSessionId::new(DeviceId::new("phone-a"), "Player"),
            offset_ms: -2500,
        };
        let json = serde_json::to_string(&command).expect("serialize media command");
        assert_eq!(
            serde_json::from_str::<MediaCommand>(&json).expect("deserialize"),
            command
        );
        assert!(!json.contains("kdeconnect"));
    }
}
