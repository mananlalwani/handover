use std::collections::BTreeSet;
use std::path::PathBuf;

use handover_core::{
    CallAction, CallCommandFailure, CallPhase, ConnectivityTransport, DeviceCommandAction,
    DeviceCommandFailure, PresentationAction, RemoteInputAction, ShareFailure, ShareStatus,
    VolumeAction,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) struct WireNotificationAction {
    pub(crate) id: String,
    pub(crate) label: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) struct WireNotification {
    pub(crate) key: String,
    pub(crate) app: String,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) clearable: bool,
    #[serde(default)]
    pub(crate) actions: Vec<WireNotificationAction>,
    #[serde(default)]
    pub(crate) reply_supported: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WirePlayback {
    Playing,
    Paused,
    Stopped,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WireControl {
    Play,
    Pause,
    PlayPause,
    Next,
    Previous,
    Seek,
    SetPosition,
}

/// Linux-to-phone media command verb. `Seek` is deliberately absent: Android
/// exposes absolute `seekTo`, which maps to `SetPosition`; relative seeks
/// have no genuine platform API, so the daemon can never route one here.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WireCommand {
    Play,
    Pause,
    PlayPause,
    Next,
    Previous,
    SetPosition,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) struct WireMediaSession {
    pub(crate) player: String,
    pub(crate) application: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) album: Option<String>,
    pub(crate) playback: WirePlayback,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) position_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) duration_ms: Option<u64>,
    #[serde(default)]
    pub(crate) controls: Vec<WireControl>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) struct WireContact {
    pub(crate) local_id: String,
    pub(crate) display_name: String,
    #[serde(default)]
    pub(crate) phones: Vec<String>,
    #[serde(default)]
    pub(crate) emails: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) photo: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) struct WireFileEntry {
    pub(crate) name: String,
    pub(crate) directory: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) size: Option<u64>,
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Message {
    Hello {
        protocol: u32,
        id: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trusted_server_id: Option<String>,
        // Hex SHA-256 commitment to the sender's fresh pairing nonce. Present
        // on every hello; required from unknown peers so the comparison code
        // binds this ceremony instead of only the long-lived certificates.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pair_commit: Option<String>,
    },
    PairOpen {
        protocol: u32,
        // Hex 16-byte nonce revealing the hello's commitment.
        nonce: String,
    },
    PairConfirm {
        protocol: u32,
        // The ceremony code the phone user approved. The server verifies it
        // against the pending candidate; a confirmation that does not repeat
        // the displayed code aborts pairing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    Paired {
        protocol: u32,
    },
    Battery {
        protocol: u32,
        percentage: u8,
        charging: bool,
    },
    Connectivity {
        protocol: u32,
        transport: ConnectivityTransport,
        validated: bool,
        metered: bool,
    },
    // Phone-to-Linux notification state. `notification_post` upserts one
    // notification; `notification_removed` retracts it; `notifications_sync`
    // carries the phone's full current list so a (re)connect reconciles stale
    // entries and advertises listener permission via `enabled`.
    NotificationPost {
        protocol: u32,
        key: String,
        app: String,
        title: String,
        body: String,
        clearable: bool,
        #[serde(default)]
        actions: Vec<WireNotificationAction>,
        #[serde(default)]
        reply_supported: bool,
    },
    NotificationRemoved {
        protocol: u32,
        key: String,
    },
    NotificationsSync {
        protocol: u32,
        enabled: bool,
        #[serde(default)]
        notifications: Vec<WireNotification>,
    },
    // Linux-to-phone direction. IPC acceptance means the command was queued
    // for the live session, not that Android confirmed the effect.
    NotificationsRequest {
        protocol: u32,
    },
    ContactsRequest {
        protocol: u32,
    },
    ContactsSync {
        protocol: u32,
        contacts: Vec<WireContact>,
    },
    ClipboardPost {
        protocol: u32,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        html: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
    },
    ClipboardSet {
        protocol: u32,
        request_id: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        html: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
    },
    ClipboardFile {
        protocol: u32,
        transfer_id: String,
        name: String,
        size: u64,
        mime: String,
    },
    ClipboardResult {
        protocol: u32,
        transfer_id: String,
        status: ShareStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<ShareFailure>,
    },
    NotificationDismiss {
        protocol: u32,
        key: String,
    },
    NotificationReply {
        protocol: u32,
        key: String,
        text: String,
    },
    NotificationAction {
        protocol: u32,
        key: String,
        action_id: String,
    },
    /// Linux-to-phone notification. The phone owns presentation and may
    /// replace an existing notification with the same request id.
    RemoteNotification {
        protocol: u32,
        request_id: String,
        app: String,
        title: String,
        body: String,
    },
    PresentationControl {
        protocol: u32,
        action: PresentationAction,
        #[serde(default)]
        delta_x: i32,
        #[serde(default)]
        delta_y: i32,
    },
    VolumeControl {
        protocol: u32,
        action: VolumeAction,
    },
    RemoteInputControl {
        protocol: u32,
        action: RemoteInputAction,
        #[serde(default)]
        delta_x: i32,
        #[serde(default)]
        delta_y: i32,
        #[serde(default)]
        button: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    CallControl {
        protocol: u32,
        request_id: String,
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        address: Option<String>,
        /// Call-state generation observed when the command was queued. The
        /// session drops the command if the phone reported newer state since;
        /// the phone independently re-checks its own current generation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generation: Option<u64>,
    },
    CallResult {
        protocol: u32,
        request_id: String,
        action: CallAction,
        accepted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure: Option<CallCommandFailure>,
    },
    CallState {
        protocol: u32,
        phase: CallPhase,
        controls: BTreeSet<handover_core::CallAction>,
        /// Monotonic counter owned by the phone. Commands are stamped with the
        /// latest observed value; the phone refuses mismatches, and the drain
        /// drops commands stamped before the newest report.
        #[serde(default)]
        generation: u64,
    },
    CallRequest {
        protocol: u32,
    },
    // Phone-to-Linux media state. `media_post` upserts one player session;
    // `media_removed` retracts it; `media_sync` carries the phone's full
    // current session list so a (re)connect reconciles stale entries.
    MediaPost {
        protocol: u32,
        player: String,
        application: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artist: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        album: Option<String>,
        playback: WirePlayback,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        #[serde(default)]
        controls: Vec<WireControl>,
    },
    MediaRemoved {
        protocol: u32,
        player: String,
    },
    MediaSync {
        protocol: u32,
        #[serde(default)]
        sessions: Vec<WireMediaSession>,
    },
    // Linux-to-phone direction. IPC acceptance means the command was queued
    // for the live session, not that Android confirmed the effect.
    MediaRequest {
        protocol: u32,
    },
    MediaControl {
        protocol: u32,
        request_id: String,
        player: String,
        action: WireCommand,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position_ms: Option<u64>,
    },
    Ring {
        protocol: u32,
        request_id: String,
    },
    UserPing {
        protocol: u32,
        request_id: String,
    },
    LockDevice {
        protocol: u32,
        request_id: String,
    },
    /// Desktop-to-phone request to hold or release a wake lock. Acceptance
    /// means Android changed the wake-lock state, not a battery guarantee.
    KeepAwake {
        protocol: u32,
        request_id: String,
        inhibit: bool,
    },
    /// Phone-to-desktop request to hold or release the desktop idle inhibitor.
    ScreensaverControl {
        protocol: u32,
        request_id: String,
        inhibit: bool,
    },
    /// Desktop-to-phone request to open the tethering settings screen.
    /// Third-party apps cannot toggle tethering directly (that requires
    /// privileged system permissions), so acceptance means the settings
    /// screen opened for the user to act, never that sharing started.
    TetheringSettings {
        protocol: u32,
        request_id: String,
    },
    /// Browse a bounded directory on the authenticated peer. Paths are
    /// interpreted by the receiving platform and never cross the trust
    /// boundary as executable input.
    FilesystemList {
        protocol: u32,
        request_id: String,
        path: String,
    },
    FilesystemEntries {
        protocol: u32,
        request_id: String,
        path: String,
        entries: Vec<WireFileEntry>,
    },
    FilesystemFailure {
        protocol: u32,
        request_id: String,
        reason: String,
    },
    CustomCommandListRequest {
        protocol: u32,
        request_id: String,
    },
    CustomCommandList {
        protocol: u32,
        request_id: String,
        names: Vec<String>,
    },
    CustomCommandRequest {
        protocol: u32,
        request_id: String,
        name: String,
    },
    CustomCommandResult {
        protocol: u32,
        request_id: String,
        name: String,
        accepted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure: Option<String>,
    },
    DeviceCommandResult {
        protocol: u32,
        request_id: String,
        action: DeviceCommandAction,
        accepted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure: Option<DeviceCommandFailure>,
    },
    ShareUrl {
        protocol: u32,
        transfer_id: String,
        url: String,
    },
    ShareFile {
        protocol: u32,
        transfer_id: String,
        name: String,
        size: u64,
        #[serde(default, skip_serializing_if = "is_false")]
        clipboard: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime: Option<String>,
        #[serde(skip)]
        path: PathBuf,
    },
    ShareResult {
        protocol: u32,
        transfer_id: String,
        status: ShareStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<ShareFailure>,
    },
    Revoke {
        protocol: u32,
    },
    Ping {
        protocol: u32,
    },
    Pong {
        protocol: u32,
    },
}

pub(crate) fn is_false(value: &bool) -> bool {
    !value
}

impl Message {
    pub(crate) fn version(&self) -> u32 {
        match self {
            Self::Hello { protocol, .. }
            | Self::PairOpen { protocol, .. }
            | Self::PairConfirm { protocol, .. }
            | Self::Paired { protocol }
            | Self::Battery { protocol, .. }
            | Self::Connectivity { protocol, .. }
            | Self::NotificationPost { protocol, .. }
            | Self::NotificationRemoved { protocol, .. }
            | Self::NotificationsSync { protocol, .. }
            | Self::NotificationsRequest { protocol }
            | Self::ContactsRequest { protocol }
            | Self::ContactsSync { protocol, .. }
            | Self::ClipboardPost { protocol, .. }
            | Self::ClipboardSet { protocol, .. }
            | Self::ClipboardFile { protocol, .. }
            | Self::ClipboardResult { protocol, .. }
            | Self::NotificationDismiss { protocol, .. }
            | Self::NotificationReply { protocol, .. }
            | Self::NotificationAction { protocol, .. }
            | Self::RemoteNotification { protocol, .. }
            | Self::PresentationControl { protocol, .. }
            | Self::VolumeControl { protocol, .. }
            | Self::RemoteInputControl { protocol, .. }
            | Self::CallControl { protocol, .. }
            | Self::CallResult { protocol, .. }
            | Self::CallState { protocol, .. }
            | Self::CallRequest { protocol }
            | Self::MediaPost { protocol, .. }
            | Self::MediaRemoved { protocol, .. }
            | Self::MediaSync { protocol, .. }
            | Self::MediaRequest { protocol }
            | Self::MediaControl { protocol, .. }
            | Self::Ring { protocol, .. }
            | Self::UserPing { protocol, .. }
            | Self::LockDevice { protocol, .. }
            | Self::KeepAwake { protocol, .. }
            | Self::ScreensaverControl { protocol, .. }
            | Self::TetheringSettings { protocol, .. }
            | Self::FilesystemList { protocol, .. }
            | Self::FilesystemEntries { protocol, .. }
            | Self::FilesystemFailure { protocol, .. }
            | Self::CustomCommandListRequest { protocol, .. }
            | Self::CustomCommandList { protocol, .. }
            | Self::CustomCommandRequest { protocol, .. }
            | Self::CustomCommandResult { protocol, .. }
            | Self::DeviceCommandResult { protocol, .. }
            | Self::ShareUrl { protocol, .. }
            | Self::ShareFile { protocol, .. }
            | Self::ShareResult { protocol, .. }
            | Self::Revoke { protocol }
            | Self::Ping { protocol }
            | Self::Pong { protocol } => *protocol,
        }
    }
}
