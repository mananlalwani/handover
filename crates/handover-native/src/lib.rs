//! Native Android transport. TLS authenticates a persistent certificate; DNS-SD only locates us.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use handover_core::{
    BatteryState, CallAction, CallCommandFailure, CallCommandResult, CallEvent, CallPhase,
    CallState, Capability, ClipboardFile, ClipboardText, ConnectivityState, ConnectivityTransport,
    Contact, ContactsEvent, Device, DeviceCommandAction, DeviceCommandFailure, DeviceCommandResult,
    DeviceEvent, DeviceId, MediaCommand, MediaControl, MediaEvent, MediaSession, MediaSessionId,
    Notification, NotificationAction, NotificationCommand, NotificationEvent, NotificationId,
    PlaybackState, PresentationAction, PresentationCommand, ReceivedShare, RemoteInputAction,
    RemoteInputCommand, ShareFailure, ShareProgress, ShareResult, ShareStatus, SharedResource,
    StateEvent, VolumeAction, VolumeCommand,
};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use openssl::asn1::Asn1Time;
use openssl::bn::{BigNum, MsbOption};
use openssl::ec::{EcGroup, EcKey};
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::{Id, PKey, Private};
use openssl::ssl::{SslAcceptor, SslMethod, SslVerifyMode, SslVersion};
use openssl::x509::{X509, X509NameBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::warn;

pub const WIRE_VERSION: u32 = 1;
pub const MAX_FRAME: usize = 64 * 1024;
const MAX_SESSIONS: usize = 16;
const MAX_SESSIONS_PER_SOURCE: usize = 8;
const MAX_ATTEMPTS_PER_SOURCE: usize = 16;
const ATTEMPT_WINDOW: Duration = Duration::from_secs(10);
const MAX_TRACKED_SOURCES: usize = 256;
const PREAUTH_READ_TIMEOUT: Duration = Duration::from_secs(5);
const LISTEN_PORT: u16 = 24837;
// Bounds for native notification fields. They keep one phone from filling the
// frame budget with a single oversized field and mirror the Android sender's
// truncation limits so both sides pin the same contract.
const MAX_NOTIFICATION_KEY: usize = 256;
const MAX_NOTIFICATION_APP: usize = 128;
const MAX_NOTIFICATION_TITLE: usize = 512;
const MAX_NOTIFICATION_BODY: usize = 4096;
const MAX_NOTIFICATION_ACTIONS: usize = 8;
const MAX_NOTIFICATION_ACTION_ID: usize = 64;
const MAX_NOTIFICATION_ACTION_LABEL: usize = 128;
const MAX_NOTIFICATIONS_PER_SYNC: usize = 64;
const MAX_NOTIFICATION_REPLY: usize = 1024;
// Bounds for native media fields. They mirror the Android sender's truncation
// limits so both sides pin the same contract; volume is never transported.
const MAX_MEDIA_PLAYER: usize = 128;
const MAX_MEDIA_APP: usize = 128;
const MAX_MEDIA_TEXT: usize = 512;
const MAX_MEDIA_SESSIONS_PER_SYNC: usize = 16;
const MAX_MEDIA_POSITION_MS: u64 = i32::MAX as u64;
const MAX_OUTBOX_PER_PEER: usize = 32;
const MAX_SHARE_SIZE: u64 = 100 * 1024 * 1024;
/// Aggregate cap for received shares: one 100 MiB file is legal, but
/// a paired endpoint must not fill the disk by repeating valid sends.
const MAX_RECEIVED_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_RECEIVED_FILES: usize = 1024;
/// Received shares older than this are garbage collected.
const RECEIVED_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const SHARE_BUFFER: usize = 32 * 1024;
const MAX_PENDING_SHARES_PER_PEER: usize = 32;
const SHARE_RESULT_TIMEOUT: Duration = Duration::from_secs(120);
const INBOUND_TRANSFER_DEADLINE: Duration = Duration::from_secs(120);

#[derive(Debug, Error)]
pub enum NativeCommandError {
    #[error("native device is disconnected")]
    Offline,
    #[error("native command queue is full")]
    QueueFull,
}

#[derive(Debug, Error)]
pub enum NativeError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("TLS: {0}")]
    Tls(#[from] openssl::error::ErrorStack),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid peer frame")]
    InvalidFrame,
    #[error("received-share quota exceeded")]
    QuotaExceeded,
    #[error("unknown pending peer or comparison code")]
    UnknownPending,
    #[error("discovery: {0}")]
    Discovery(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PendingPeer {
    pub id: String,
    pub name: String,
    pub code: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct PeerFile {
    peers: BTreeMap<String, Peer>,
}

struct Candidate {
    peer: Peer,
    code: String,
    commit: String,
    approved: bool,
    created: Instant,
}
struct Runtime {
    peers: PeerFile,
    pending: BTreeMap<String, Candidate>,
    active: BTreeMap<String, TcpStream>,
    sessions: usize,
    source_admissions: BTreeMap<IpAddr, SourceAdmission>,
    connecting: BTreeSet<String>,
    batteries: BTreeMap<String, BatteryState>,
    connectivity: BTreeMap<String, ConnectivityState>,
    // Native notification state, owned per paired peer. Keys are the Android
    // notification keys advertised as `local_id` in the normalized model.
    notif_enabled: BTreeMap<String, bool>,
    notif_keys: BTreeMap<String, BTreeSet<String>>,
    // Native media state, owned per paired peer. Players are the Android
    // package names advertised as `player_id` in the normalized model.
    media_enabled: BTreeMap<String, bool>,
    media_players: BTreeMap<String, BTreeSet<String>>,
    // Queued Linux-to-phone notification commands, drained by the owning
    // session thread. Bounded per peer; IPC reports acceptance, not delivery.
    outbox: BTreeMap<String, Vec<Message>>,
    pending_shares: BTreeMap<String, BTreeMap<String, Instant>>,
    // Latest call-state generation reported by each peer. A queued call
    // command stamped with an older generation is stale: a new call may have
    // started or ended since the user acted, so the command is dropped.
    call_generations: BTreeMap<String, u64>,
    pending_call_results: BTreeMap<String, BTreeMap<String, CallAction>>,
    // Peers that asked the desktop to stay awake. Cleared on disconnect so a
    // dead phone cannot hold the inhibitor past its session.
    screensaver_requests: BTreeSet<String>,
    /// In-flight received-share reservations (bytes, files). Disk scans
    /// cannot see concurrent transfers, so each transfer reserves
    /// before streaming and releases on failure; completed files stay
    /// counted by later scans.
    quota_reserved: (u64, usize),
}

#[derive(Default)]
struct SourceAdmission {
    active: usize,
    attempts: VecDeque<Instant>,
}

fn admit_source(inner: &mut Runtime, source: IpAddr, now: Instant) -> bool {
    for admission in inner.source_admissions.values_mut() {
        admission
            .attempts
            .retain(|started| now.saturating_duration_since(*started) < ATTEMPT_WINDOW);
    }
    inner
        .source_admissions
        .retain(|_, admission| admission.active > 0 || !admission.attempts.is_empty());
    if !inner.source_admissions.contains_key(&source)
        && inner.source_admissions.len() >= MAX_TRACKED_SOURCES
    {
        return false;
    }
    let admission = inner.source_admissions.entry(source).or_default();
    if admission.active >= MAX_SESSIONS_PER_SOURCE
        || admission.attempts.len() >= MAX_ATTEMPTS_PER_SOURCE
    {
        return false;
    }
    admission.active += 1;
    admission.attempts.push_back(now);
    true
}

fn release_source(inner: &mut Runtime, source: IpAddr) {
    if let Some(admission) = inner.source_admissions.get_mut(&source) {
        admission.active = admission.active.saturating_sub(1);
    }
}

#[derive(Clone)]
pub struct NativeBackend {
    inner: Arc<Mutex<Runtime>>,
    directory: PathBuf,
    certificate: X509,
    key: PKey<Private>,
    id: String,
}

// Serializes session teardown with peer admission so an old disconnect cannot
// overwrite a replacement connection's presence.
struct Session<'a> {
    backend: &'a NativeBackend,
    peer: Peer,
    event: &'a Arc<dyn Fn(StateEvent) + Send + Sync>,
    published: bool,
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        // Events are collected under the lock and emitted after it drops:
        // callbacks may call back into the backend (screensaver refresh does),
        // and emitting under the lock would deadlock the session thread.
        let pending_events = {
            let mut inner = self.backend.inner.lock().unwrap();
            inner.connecting.remove(&self.peer.id);
            inner.pending.remove(&self.peer.id);
            if !self.published {
                return;
            }
            let mut pending_events = Vec::new();
            inner.active.remove(&self.peer.id);
            let abandoned = inner.outbox.remove(&self.peer.id).unwrap_or_default();
            for message in &abandoned {
                delete_clipboard_temp(message);
            }
            let pending = inner
                .pending_shares
                .remove(&self.peer.id)
                .unwrap_or_default();
            for transfer_id in pending.into_keys() {
                pending_events.push(StateEvent::ShareResult(ShareResult {
                    device_id: DeviceId::new(format!("native:{}", self.peer.id)),
                    transfer_id,
                    status: ShareStatus::Failed,
                    reason: Some(ShareFailure::Disconnected),
                }));
            }
            let removed_keys = inner.notif_keys.remove(&self.peer.id).unwrap_or_default();
            inner.notif_enabled.remove(&self.peer.id);
            let removed_players = inner
                .media_players
                .remove(&self.peer.id)
                .unwrap_or_default();
            inner.media_enabled.remove(&self.peer.id);
            inner.connectivity.remove(&self.peer.id);
            inner.pending_call_results.remove(&self.peer.id);
            inner.screensaver_requests.remove(&self.peer.id);
            let device_id = device(&self.peer, false, None, false, false).id;
            let mut removals: Vec<NotificationId> = removed_keys
                .into_iter()
                .map(|key| NotificationId::new(device_id.clone(), key))
                .collect();
            removals.sort();
            let mut media_removals: Vec<MediaSessionId> = removed_players
                .into_iter()
                .map(|player| MediaSessionId::new(device_id.clone(), player))
                .collect();
            media_removals.sort();
            let event = if inner.peers.peers.contains_key(&self.peer.id) {
                DeviceEvent::Updated(device(
                    &self.peer,
                    false,
                    inner.batteries.get(&self.peer.id).cloned(),
                    false,
                    false,
                ))
            } else {
                DeviceEvent::Removed(device(&self.peer, false, None, false, false).id)
            };
            // Clearing per-peer notification keys here keeps a disconnect from
            // leaving ghost state while a replacement connection resyncs from
            // the phone's current list. KDE-derived entries are untouched:
            // their device IDs never carry the `native:` prefix.
            for id in removals {
                pending_events.push(StateEvent::Notification(NotificationEvent::Removed(id)));
            }
            // Same ownership rule for media sessions: the disconnect drops the
            // peer's native players so a reconnect resyncs from current phone
            // state. `handoverd` also strips sessions of disconnected devices,
            // and the store dedupes, so a double removal is harmless.
            for id in media_removals {
                pending_events.push(StateEvent::Media(MediaEvent::Removed(id)));
            }
            pending_events.push(StateEvent::Call(CallEvent::Removed(device_id)));
            pending_events.push(StateEvent::Device(event));
            pending_events
        };
        for event in pending_events {
            (self.event)(event);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct WireNotificationAction {
    id: String,
    label: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct WireNotification {
    key: String,
    app: String,
    title: String,
    body: String,
    clearable: bool,
    #[serde(default)]
    actions: Vec<WireNotificationAction>,
    #[serde(default)]
    reply_supported: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum WirePlayback {
    Playing,
    Paused,
    Stopped,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum WireControl {
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
enum WireCommand {
    Play,
    Pause,
    PlayPause,
    Next,
    Previous,
    SetPosition,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct WireMediaSession {
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
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct WireContact {
    local_id: String,
    display_name: String,
    #[serde(default)]
    phones: Vec<String>,
    #[serde(default)]
    emails: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    photo: Option<String>,
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Message {
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

fn is_false(value: &bool) -> bool {
    !value
}

impl Message {
    fn version(&self) -> u32 {
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

/// Temp path owned by a queued clipboard share, if any. Only
/// `clipboard: true` entries name daemon-owned temp files; user files
/// must never be deleted by queue cleanup.
fn clipboard_temp_path(message: &Message) -> Option<&PathBuf> {
    match message {
        Message::ShareFile {
            clipboard: true,
            path,
            ..
        } => Some(path),
        _ => None,
    }
}

/// Delete a queued clipboard share's temp file. Every terminal path
/// for a queued entry (streamed, cancelled, expired, timed out,
/// disconnected, send-failed) must call this.
fn delete_clipboard_temp(message: &Message) {
    if let Some(path) = clipboard_temp_path(message) {
        let _ = fs::remove_file(path);
    }
}

/// Reservation against the received-share quota. Dropping an
/// uncommitted reservation releases it; a completed transfer commits
/// it, leaving the on-disk file to future scans.
struct QuotaReservation {
    backend: NativeBackend,
    bytes: u64,
    committed: bool,
}

impl QuotaReservation {
    fn commit(mut self) {
        let mut inner = self.backend.inner.lock().unwrap();
        inner.quota_reserved.0 = inner.quota_reserved.0.saturating_sub(self.bytes);
        inner.quota_reserved.1 = inner.quota_reserved.1.saturating_sub(1);
        self.committed = true;
    }
}

impl Drop for QuotaReservation {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let mut inner = self.backend.inner.lock().unwrap();
        inner.quota_reserved.0 = inner.quota_reserved.0.saturating_sub(self.bytes);
        inner.quota_reserved.1 = inner.quota_reserved.1.saturating_sub(1);
    }
}

impl NativeBackend {
    pub fn open(directory: PathBuf) -> Result<Self, NativeError> {
        fs::create_dir_all(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        let key_path = directory.join("identity.key");
        let cert_path = directory.join("identity.crt");
        let (key, certificate) = if key_path.exists() && cert_path.exists() {
            (
                PKey::private_key_from_pem(&fs::read(&key_path)?)?,
                X509::from_pem(&fs::read(&cert_path)?)?,
            )
        } else {
            let (key, cert) = generate_identity()?;
            write_private(&key_path, &key.private_key_to_pem_pkcs8()?)?;
            write_private(&cert_path, &cert.to_pem()?)?;
            (key, cert)
        };
        let id = fingerprint(&certificate)?;
        let peers_path = directory.join("peers.json");
        let peers = if peers_path.exists() {
            serde_json::from_slice(&fs::read(peers_path)?)?
        } else {
            PeerFile::default()
        };
        Ok(Self {
            inner: Arc::new(Mutex::new(Runtime {
                peers,
                pending: BTreeMap::new(),
                active: BTreeMap::new(),
                sessions: 0,
                source_admissions: BTreeMap::new(),
                connecting: BTreeSet::new(),
                batteries: BTreeMap::new(),
                connectivity: BTreeMap::new(),
                notif_enabled: BTreeMap::new(),
                notif_keys: BTreeMap::new(),
                media_enabled: BTreeMap::new(),
                media_players: BTreeMap::new(),
                outbox: BTreeMap::new(),
                pending_shares: BTreeMap::new(),
                call_generations: BTreeMap::new(),
                pending_call_results: BTreeMap::new(),
                screensaver_requests: BTreeSet::new(),
                quota_reserved: (0, 0),
            })),
            directory,
            certificate,
            key,
            id,
        })
    }

    pub fn default_directory() -> Result<PathBuf, NativeError> {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "HOME or XDG_STATE_HOME is required",
                )
            })?;
        Ok(base.join("handover/native"))
    }

    pub fn identity(&self) -> &str {
        &self.id
    }
    pub fn peers(&self) -> Vec<Peer> {
        self.inner
            .lock()
            .unwrap()
            .peers
            .peers
            .values()
            .cloned()
            .collect()
    }
    pub fn remembered_devices(&self) -> Vec<Device> {
        self.peers()
            .iter()
            .map(|peer| device(peer, false, None, false, false))
            .collect()
    }
    pub fn pending(&self) -> Vec<PendingPeer> {
        let mut inner = self.inner.lock().unwrap();
        inner
            .pending
            .retain(|_, candidate| candidate.created.elapsed() < Duration::from_secs(120));
        inner
            .pending
            .values()
            .map(|c| PendingPeer {
                id: c.peer.id.clone(),
                name: c.peer.name.clone(),
                code: c.code.clone(),
            })
            .collect()
    }
    pub fn approve(&self, id: &str, code: &str) -> Result<(), NativeError> {
        let mut inner = self.inner.lock().unwrap();
        inner
            .pending
            .retain(|_, candidate| candidate.created.elapsed() < Duration::from_secs(120));
        let candidate = inner
            .pending
            .get_mut(id)
            .ok_or(NativeError::UnknownPending)?;
        if candidate.code.is_empty() || candidate.code != code {
            // The code only exists after both sides reveal their pairing
            // nonces; there is nothing to approve before that.
            return Err(NativeError::UnknownPending);
        }
        candidate.approved = true;
        Ok(())
    }
    pub fn unpair(&self, id: &str) -> Result<bool, NativeError> {
        let mut inner = self.inner.lock().unwrap();
        inner.pending.remove(id);
        let mut peers = inner.peers.clone();
        let removed = peers.peers.remove(id).is_some();
        if removed {
            self.save_peers(&peers)?;
            inner.peers = peers;
            inner.batteries.remove(id);
            inner.connectivity.remove(id);
        }
        inner.notif_enabled.remove(id);
        inner.notif_keys.remove(id);
        inner.media_enabled.remove(id);
        inner.media_players.remove(id);
        if let Some(abandoned) = inner.outbox.remove(id) {
            for message in &abandoned {
                delete_clipboard_temp(message);
            }
        }
        if let Some(stream) = inner.active.remove(id) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        Ok(removed)
    }

    /// Queue a validated notification command for the live native session.
    /// Success means the command was accepted for delivery, not that Android
    /// confirmed the dismissal, action, or reply. The owning session thread
    /// drains the queue; a disconnected peer reports [`NativeCommandError::Offline`].
    pub fn execute_notification(
        &self,
        peer_id: &str,
        command: &NotificationCommand,
    ) -> Result<(), NativeCommandError> {
        let message = match command {
            NotificationCommand::Dismiss { notification_id } => Message::NotificationDismiss {
                protocol: WIRE_VERSION,
                key: notification_id.local_id.clone(),
            },
            NotificationCommand::Reply {
                notification_id,
                text,
            } => {
                if text.len() > MAX_NOTIFICATION_REPLY {
                    return Err(NativeCommandError::QueueFull);
                }
                Message::NotificationReply {
                    protocol: WIRE_VERSION,
                    key: notification_id.local_id.clone(),
                    text: text.clone(),
                }
            }
            NotificationCommand::InvokeAction {
                notification_id,
                action_id,
            } => Message::NotificationAction {
                protocol: WIRE_VERSION,
                key: notification_id.local_id.clone(),
                action_id: action_id.clone(),
            },
        };
        let mut inner = self.inner.lock().unwrap();
        if !inner.active.contains_key(peer_id) {
            return Err(NativeCommandError::Offline);
        }
        let queue = inner.outbox.entry(peer_id.to_owned()).or_default();
        if queue.len() >= MAX_OUTBOX_PER_PEER {
            return Err(NativeCommandError::QueueFull);
        }
        queue.push(message);
        Ok(())
    }

    pub fn send_notification(
        &self,
        peer_id: &str,
        app: String,
        title: String,
        body: String,
    ) -> Result<(), NativeCommandError> {
        if app.is_empty()
            || app.len() > MAX_NOTIFICATION_APP
            || title.len() > MAX_NOTIFICATION_TITLE
            || body.len() > MAX_NOTIFICATION_BODY
            || app.chars().any(char::is_control)
            || title.chars().any(char::is_control)
            || body.chars().any(char::is_control)
        {
            return Err(NativeCommandError::QueueFull);
        }
        let request_id = new_transfer_id().map_err(|_| NativeCommandError::QueueFull)?;
        self.queue_simple(
            peer_id,
            Message::RemoteNotification {
                protocol: WIRE_VERSION,
                request_id,
                app,
                title,
                body,
            },
        )
    }

    pub fn presentation_control(
        &self,
        peer_id: &str,
        command: &PresentationCommand,
    ) -> Result<(), NativeCommandError> {
        if command.delta_x.abs() > 2000 || command.delta_y.abs() > 2000 {
            return Err(NativeCommandError::QueueFull);
        }
        self.queue_simple(
            peer_id,
            Message::PresentationControl {
                protocol: WIRE_VERSION,
                action: command.action,
                delta_x: command.delta_x,
                delta_y: command.delta_y,
            },
        )
    }

    pub fn volume_control(
        &self,
        peer_id: &str,
        command: &VolumeCommand,
    ) -> Result<(), NativeCommandError> {
        self.queue_simple(
            peer_id,
            Message::VolumeControl {
                protocol: WIRE_VERSION,
                action: command.action,
            },
        )
    }

    pub fn remote_input(
        &self,
        peer_id: &str,
        command: &RemoteInputCommand,
    ) -> Result<(), NativeCommandError> {
        if command.delta_x.abs() > 2000
            || command.delta_y.abs() > 2000
            || command.button > 5
            || command.text.as_ref().is_some_and(|text| text.len() > 512)
        {
            return Err(NativeCommandError::QueueFull);
        }
        self.queue_simple(
            peer_id,
            Message::RemoteInputControl {
                protocol: WIRE_VERSION,
                action: command.action,
                delta_x: command.delta_x,
                delta_y: command.delta_y,
                button: command.button,
                text: command.text.clone(),
            },
        )
    }

    pub fn clipboard_set(
        &self,
        peer_id: &str,
        text: &str,
        html: Option<String>,
        uri: Option<String>,
    ) -> Result<(), NativeCommandError> {
        if text.len() > 32 * 1024 {
            return Err(NativeCommandError::QueueFull);
        }
        let request_id = new_transfer_id().map_err(|_| NativeCommandError::QueueFull)?;
        self.queue_simple(
            peer_id,
            Message::ClipboardSet {
                protocol: WIRE_VERSION,
                request_id,
                text: text.to_owned(),
                html,
                uri,
            },
        )
    }

    /// Ask the live session to request a fresh notification sync. Returns
    /// false when the peer has no active session.
    pub fn request_notifications_sync(&self, peer_id: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if !inner.active.contains_key(peer_id) {
            return false;
        }
        let queue = inner.outbox.entry(peer_id.to_owned()).or_default();
        if queue.len() >= MAX_OUTBOX_PER_PEER {
            return false;
        }
        queue.push(Message::NotificationsRequest {
            protocol: WIRE_VERSION,
        });
        true
    }

    pub fn request_contacts_sync(&self, peer_id: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if !inner.active.contains_key(peer_id) {
            return false;
        }
        let queue = inner.outbox.entry(peer_id.to_owned()).or_default();
        if queue.len() >= MAX_OUTBOX_PER_PEER {
            return false;
        }
        queue.push(Message::ContactsRequest {
            protocol: WIRE_VERSION,
        });
        true
    }

    pub fn call_control(
        &self,
        peer_id: &str,
        action: &str,
        address: Option<String>,
        generation: Option<u64>,
    ) -> Result<String, NativeCommandError> {
        if !matches!(action, "place" | "answer" | "decline" | "hangup") {
            return Err(NativeCommandError::QueueFull);
        }
        let mut inner = self.inner.lock().unwrap();
        if !inner.active.contains_key(peer_id) {
            return Err(NativeCommandError::Offline);
        }
        let queue = inner.outbox.entry(peer_id.to_owned()).or_default();
        if queue.len() >= MAX_OUTBOX_PER_PEER {
            return Err(NativeCommandError::QueueFull);
        }
        let request_id = new_transfer_id().map_err(|_| NativeCommandError::QueueFull)?;
        queue.push(Message::CallControl {
            protocol: WIRE_VERSION,
            request_id: request_id.clone(),
            action: action.into(),
            address,
            generation,
        });
        Ok(request_id)
    }

    /// Queue a validated media command for the live native session. Success
    /// means the command was accepted for delivery, not that Android changed
    /// playback; authoritative state arrives as a later media update. The
    /// daemon validates the command against current state before routing, so
    /// a relative `Seek` can never reach here: native sessions never
    /// advertise it.
    pub fn execute_media(
        &self,
        peer_id: &str,
        command: &MediaCommand,
    ) -> Result<(), NativeCommandError> {
        let (player, action, position_ms) = match command {
            MediaCommand::Play { id } => (id.player_id.clone(), WireCommand::Play, None),
            MediaCommand::Pause { id } => (id.player_id.clone(), WireCommand::Pause, None),
            MediaCommand::PlayPause { id } => (id.player_id.clone(), WireCommand::PlayPause, None),
            MediaCommand::Next { id } => (id.player_id.clone(), WireCommand::Next, None),
            MediaCommand::Previous { id } => (id.player_id.clone(), WireCommand::Previous, None),
            MediaCommand::SetPosition { id, position_ms } => (
                id.player_id.clone(),
                WireCommand::SetPosition,
                Some(*position_ms),
            ),
            MediaCommand::Seek { .. } => return Err(NativeCommandError::QueueFull),
        };
        let mut inner = self.inner.lock().unwrap();
        if !inner.active.contains_key(peer_id) {
            return Err(NativeCommandError::Offline);
        }
        let queue = inner.outbox.entry(peer_id.to_owned()).or_default();
        if queue.len() >= MAX_OUTBOX_PER_PEER {
            return Err(NativeCommandError::QueueFull);
        }
        let request_id = new_transfer_id().map_err(|_| NativeCommandError::QueueFull)?;
        queue.push(Message::MediaControl {
            protocol: WIRE_VERSION,
            request_id,
            player,
            action,
            position_ms,
        });
        Ok(())
    }

    /// Ask the live session to report its current media sessions. Returns
    /// false when the peer has no active session.
    pub fn request_media_sync(&self, peer_id: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if !inner.active.contains_key(peer_id) {
            return false;
        }
        let queue = inner.outbox.entry(peer_id.to_owned()).or_default();
        if queue.len() >= MAX_OUTBOX_PER_PEER {
            return false;
        }
        queue.push(Message::MediaRequest {
            protocol: WIRE_VERSION,
        });
        true
    }

    /// Queue a user-visible liveness ping for a live native session.
    pub fn ping(&self, peer_id: &str) -> Result<(), NativeCommandError> {
        let request_id = new_transfer_id().map_err(|_| NativeCommandError::QueueFull)?;
        self.queue_simple(
            peer_id,
            Message::UserPing {
                protocol: WIRE_VERSION,
                request_id,
            },
        )
    }

    /// Queue a request for the phone to ring and vibrate.
    pub fn ring(&self, peer_id: &str) -> Result<(), NativeCommandError> {
        let request_id = new_transfer_id().map_err(|_| NativeCommandError::QueueFull)?;
        self.queue_simple(
            peer_id,
            Message::Ring {
                protocol: WIRE_VERSION,
                request_id,
            },
        )
    }

    /// Queue a request for the phone to lock itself. Android applies its
    /// device-admin permission check before executing the request.
    pub fn lock_device(&self, peer_id: &str) -> Result<(), NativeCommandError> {
        let request_id = new_transfer_id().map_err(|_| NativeCommandError::QueueFull)?;
        self.queue_simple(
            peer_id,
            Message::LockDevice {
                protocol: WIRE_VERSION,
                request_id,
            },
        )
    }

    /// Queue a request for the phone to hold (`inhibit: true`) or release its
    /// wake lock. Android reports the applied state as a device command result.
    pub fn keep_awake(&self, peer_id: &str, inhibit: bool) -> Result<(), NativeCommandError> {
        let request_id = new_transfer_id().map_err(|_| NativeCommandError::QueueFull)?;
        self.queue_simple(
            peer_id,
            Message::KeepAwake {
                protocol: WIRE_VERSION,
                request_id,
                inhibit,
            },
        )
    }

    /// Peer IDs currently asking the desktop to stay awake.
    pub fn phone_screensaver_requests(&self) -> Vec<String> {
        self.inner
            .lock()
            .unwrap()
            .screensaver_requests
            .iter()
            .cloned()
            .collect()
    }

    /// Queue a request for the phone to open its tethering settings screen.
    /// Acceptance means the screen opened, never that sharing started: the
    /// platform reserves actual tethering control for system apps.
    pub fn tethering_settings(&self, peer_id: &str) -> Result<(), NativeCommandError> {
        let request_id = new_transfer_id().map_err(|_| NativeCommandError::QueueFull)?;
        self.queue_simple(
            peer_id,
            Message::TetheringSettings {
                protocol: WIRE_VERSION,
                request_id,
            },
        )
    }

    fn queue_simple(&self, peer_id: &str, message: Message) -> Result<(), NativeCommandError> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.active.contains_key(peer_id) {
            return Err(NativeCommandError::Offline);
        }
        let queue = inner.outbox.entry(peer_id.to_owned()).or_default();
        if queue.len() >= MAX_OUTBOX_PER_PEER {
            return Err(NativeCommandError::QueueFull);
        }
        queue.push(message);
        Ok(())
    }

    /// Accept one share for a live paired peer. Delivery is asynchronous; a
    /// later disconnect or I/O failure can prevent completion.
    pub fn share_url(&self, peer_id: &str, url: String) -> Result<String, NativeCommandError> {
        if !valid_share_url(&url) {
            return Err(NativeCommandError::QueueFull);
        }
        let transfer_id = new_transfer_id().map_err(|_| NativeCommandError::QueueFull)?;
        self.queue_share(
            peer_id,
            Message::ShareUrl {
                protocol: WIRE_VERSION,
                transfer_id: transfer_id.clone(),
                url,
            },
            transfer_id,
        )
        .map_err(|error| error.0)
    }

    pub fn share_file(&self, peer_id: &str, path: PathBuf) -> Result<String, NativeCommandError> {
        let file = File::open(&path).map_err(|_| NativeCommandError::QueueFull)?;
        let metadata = file.metadata().map_err(|_| NativeCommandError::QueueFull)?;
        if !metadata.is_file() {
            return Err(NativeCommandError::QueueFull);
        }
        let size = metadata.len();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| safe_share_name(n) && n.len() <= 255)
            .ok_or(NativeCommandError::QueueFull)?
            .to_owned();
        if size > MAX_SHARE_SIZE {
            return Err(NativeCommandError::QueueFull);
        }
        let transfer_id = new_transfer_id().map_err(|_| NativeCommandError::QueueFull)?;
        self.queue_share(
            peer_id,
            Message::ShareFile {
                protocol: WIRE_VERSION,
                transfer_id: transfer_id.clone(),
                name,
                size,
                clipboard: false,
                mime: None,
                path,
            },
            transfer_id,
        )
        .map_err(|error| error.0)
    }

    /// Queue a Wayland clipboard image captured to a temp file. Takes
    /// ownership of the temp path on every outcome: validation and
    /// queue failures delete it, queued entries delete it on every
    /// terminal path (streamed, cancelled, expired, disconnected).
    pub fn clipboard_file(
        &self,
        peer_id: &str,
        path: PathBuf,
        mime: String,
    ) -> Result<String, NativeCommandError> {
        let rejected = |path: PathBuf| {
            let _ = fs::remove_file(&path);
            NativeCommandError::QueueFull
        };
        if mime.is_empty() || mime.len() > 128 {
            return Err(rejected(path));
        }
        let file = File::open(&path).map_err(|_| rejected(path.clone()))?;
        let metadata = file.metadata().map_err(|_| rejected(path.clone()))?;
        if !metadata.is_file() || metadata.len() > 10 * 1024 * 1024 {
            return Err(rejected(path));
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| safe_share_name(name) && name.len() <= 255)
            .map(str::to_owned);
        let Some(name) = name else {
            return Err(rejected(path));
        };
        let transfer_id = new_transfer_id().map_err(|_| rejected(path.clone()))?;
        self.queue_share(
            peer_id,
            Message::ShareFile {
                protocol: WIRE_VERSION,
                transfer_id: transfer_id.clone(),
                name,
                size: metadata.len(),
                clipboard: true,
                mime: Some(mime),
                path,
            },
            transfer_id,
        )
        .map_err(|error| {
            delete_clipboard_temp(&error.1);
            error.0
        })
    }

    /// Cancel a queued share that has not started streaming. Returns true
    /// when a queued command or pending result entry was removed. A transfer
    /// already being written to the session stream cannot be recalled; its
    /// receiver result stands.
    pub fn cancel_share(&self, peer_id: &str, transfer_id: &str) -> bool {
        if !valid_transfer_id(transfer_id) {
            return false;
        }
        let mut inner = self.inner.lock().unwrap();
        let mut removed = false;
        if let Some(queue) = inner.outbox.get_mut(peer_id) {
            let before = queue.len();
            let mut temps = Vec::new();
            queue.retain(|message| {
                let drop_it = match message {
                    Message::ShareUrl {
                        transfer_id: id, ..
                    }
                    | Message::ShareFile {
                        transfer_id: id, ..
                    } => id == transfer_id,
                    _ => false,
                };
                if drop_it {
                    temps.extend(clipboard_temp_path(message).cloned());
                }
                !drop_it
            });
            removed = queue.len() != before;
            drop(inner);
            for temp in temps {
                let _ = fs::remove_file(temp);
            }
            let mut inner = self.inner.lock().unwrap();
            if let Some(pending) = inner.pending_shares.get_mut(peer_id) {
                removed |= pending.remove(transfer_id).is_some();
            }
        } else if let Some(pending) = inner.pending_shares.get_mut(peer_id) {
            removed |= pending.remove(transfer_id).is_some();
        }
        removed
    }

    fn queue_share(
        &self,
        peer_id: &str,
        message: Message,
        transfer_id: String,
    ) -> Result<String, Box<(NativeCommandError, Message)>> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.active.contains_key(peer_id) {
            return Err(Box::new((NativeCommandError::Offline, message)));
        }
        let queue = inner.outbox.entry(peer_id.to_owned()).or_default();
        if queue.len() >= MAX_OUTBOX_PER_PEER {
            return Err(Box::new((NativeCommandError::QueueFull, message)));
        }
        let pending = inner.pending_shares.entry(peer_id.to_owned()).or_default();
        if pending.len() >= MAX_PENDING_SHARES_PER_PEER || pending.contains_key(&transfer_id) {
            return Err(Box::new((NativeCommandError::QueueFull, message)));
        }
        pending.insert(transfer_id.clone(), Instant::now());
        let queue = inner.outbox.entry(peer_id.to_owned()).or_default();
        queue.push(message);
        Ok(transfer_id)
    }

    /// Enforce the aggregate received-share quota before accepting one
    /// more file: sweep entries older than the TTL, then oldest-first
    /// until the incoming size fits. Refuses when even an empty
    /// directory cannot fit the file. The check and the reservation
    /// are atomic under the runtime lock: concurrent transfers cannot
    /// all observe spare capacity and overshoot together.
    fn reserve_received_quota(
        &self,
        directory: &Path,
        incoming: u64,
    ) -> Result<QuotaReservation, NativeError> {
        use std::time::SystemTime;
        if incoming > MAX_RECEIVED_BYTES {
            return Err(NativeError::InvalidFrame);
        }
        let mut inner = self.inner.lock().unwrap();
        let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
        let mut subdirs: Vec<PathBuf> = Vec::new();
        let mut walk: Vec<PathBuf> = vec![directory.to_path_buf()];
        while let Some(directory) = walk.pop() {
            let Ok(children) = fs::read_dir(&directory) else {
                continue;
            };
            for child in children.flatten() {
                let Ok(kind) = child.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    subdirs.push(child.path());
                    walk.push(child.path());
                } else if kind.is_file() {
                    let Ok(metadata) = child.metadata() else {
                        continue;
                    };
                    files.push((
                        metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                        metadata.len(),
                        child.path(),
                    ));
                }
            }
        }
        let now = SystemTime::now();
        files.retain(|(modified, _, path)| {
            if now
                .duration_since(*modified)
                .is_ok_and(|age| age > RECEIVED_TTL)
            {
                let _ = fs::remove_file(path);
                return false;
            }
            true
        });
        files.sort_by_key(|entry| entry.0);
        let mut total: u64 = files.iter().map(|entry| entry.1).sum();
        total = total.saturating_add(inner.quota_reserved.0);
        let mut count = files.len() + inner.quota_reserved.1;
        for (_, len, path) in &files {
            if total.saturating_add(incoming) <= MAX_RECEIVED_BYTES && count < MAX_RECEIVED_FILES {
                break;
            }
            if fs::remove_file(path).is_ok() {
                total = total.saturating_sub(*len);
                count = count.saturating_sub(1);
            }
        }
        // Active `.partial` files of concurrent transfers are counted
        // above; the reservation below covers this transfer's own.
        if total.saturating_add(incoming) > MAX_RECEIVED_BYTES || count >= MAX_RECEIVED_FILES {
            return Err(NativeError::QuotaExceeded);
        }
        for subdir in subdirs {
            // Succeeds only when every child is gone.
            let _ = fs::remove_dir(subdir);
        }
        inner.quota_reserved.0 = inner.quota_reserved.0.saturating_add(incoming);
        inner.quota_reserved.1 += 1;
        Ok(QuotaReservation {
            backend: self.clone(),
            bytes: incoming,
            committed: false,
        })
    }

    fn receive_share_file<R: Read>(
        &self,
        input: &mut R,
        name: &str,
        size: u64,
    ) -> Result<PathBuf, NativeError> {
        self.receive_share_file_until(
            input,
            name,
            size,
            Instant::now() + INBOUND_TRANSFER_DEADLINE,
        )
    }

    fn receive_share_file_until<R: Read>(
        &self,
        input: &mut R,
        name: &str,
        size: u64,
        deadline: Instant,
    ) -> Result<PathBuf, NativeError> {
        if !safe_share_name(name) || name.len() > 255 || size > MAX_SHARE_SIZE {
            return Err(NativeError::InvalidFrame);
        }
        let directory = self.directory.join("received");
        fs::create_dir_all(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        let reservation = self.reserve_received_quota(&directory, size)?;
        let mut random = [0u8; 8];
        openssl::rand::rand_bytes(&mut random)?;
        let suffix = random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let transfer_directory = directory.join(suffix);
        fs::create_dir(&transfer_directory)?;
        let destination = transfer_directory.join(name);
        let partial = transfer_directory.join(".partial");
        let mut guard = PartialShare(partial.clone(), transfer_directory);
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)?;
        output.set_permissions(fs::Permissions::from_mode(0o600))?;
        let mut remaining = size;
        let mut buffer = [0u8; SHARE_BUFFER];
        while remaining > 0 {
            let amount = usize::try_from(remaining.min(SHARE_BUFFER as u64)).unwrap();
            read_exact_until(input, &mut buffer[..amount], deadline)?;
            output.write_all(&buffer[..amount])?;
            remaining -= amount as u64;
        }
        output.sync_all()?;
        drop(output);
        fs::rename(&partial, &destination)?;
        guard.0 = PathBuf::new();
        // The file is on disk and counted by future scans; release
        // the in-flight reservation without double counting.
        reservation.commit();
        Ok(destination)
    }
    fn save_peers(&self, peers: &PeerFile) -> Result<(), NativeError> {
        let path = self.directory.join("peers.json");
        let tmp = self.directory.join("peers.json.tmp");
        write_private(&tmp, &serde_json::to_vec(peers)?)?;
        fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn run(self, event: Arc<dyn Fn(StateEvent) + Send + Sync>) -> Result<(), NativeError> {
        self.run_on_port(LISTEN_PORT, event)
    }

    /// Runs the advertised listener on an explicit port. Debug smoke tests use
    /// this to exercise a second daemon without disturbing the live service.
    #[doc(hidden)]
    pub fn run_on_port(
        self,
        port: u16,
        event: Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        let port = listener.local_addr()?.port();
        // Keep the advertisement alive for the lifetime of the listener.
        let _discovery = advertise(port, &self.id)?;
        self.serve(listener, event)
    }

    /// Serves the native protocol on an already-bound listener without DNS-SD
    /// advertisement. The pairing/TLS interop harness and integration tests
    /// use this with an ephemeral port; production callers use [`Self::run`].
    pub fn serve(
        self,
        listener: TcpListener,
        event: Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        for incoming in listener.incoming() {
            let stream = match incoming {
                Ok(stream) => stream,
                // Transient accept failures (fd exhaustion, aborted
                // handshakes) must not end the listener permanently.
                Err(error) => {
                    tracing::debug!(%error, "native accept failed; listening on");
                    continue;
                }
            };
            let Ok(source) = stream.peer_addr().map(|address| address.ip()) else {
                continue;
            };
            let mut inner = self.inner.lock().unwrap();
            if inner.sessions >= MAX_SESSIONS || !admit_source(&mut inner, source, Instant::now()) {
                continue;
            }
            inner.sessions += 1;
            drop(inner);
            let backend = self.clone();
            let event = event.clone();
            std::thread::spawn(move || {
                if let Err(error) = backend.handle(stream, &event) {
                    tracing::debug!(%error, "native connection ended");
                }
                let mut inner = backend.inner.lock().unwrap();
                inner.sessions -= 1;
                release_source(&mut inner, source);
            });
        }
        Ok(())
    }

    fn handle(
        &self,
        stream: TcpStream,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        // TLS and the first hello are unauthenticated. Keep each socket read
        // bounded while the admission controls cap concurrent attempts.
        stream.set_read_timeout(Some(PREAUTH_READ_TIMEOUT))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut builder = SslAcceptor::mozilla_modern_v5(SslMethod::tls())?;
        builder.set_min_proto_version(Some(SslVersion::TLS1_3))?;
        builder.set_certificate(&self.certificate)?;
        builder.set_private_key(&self.key)?;
        builder.set_verify_callback(
            SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT,
            |_preverified, _context| true,
        );
        let acceptor = builder.build();
        let mut tls = match acceptor.accept(stream) {
            Ok(tls) => tls,
            Err(error) => {
                warn!(%error, "native TLS handshake failed");
                return Err(NativeError::InvalidFrame);
            }
        };
        tls.get_ref()
            .set_read_timeout(Some(Duration::from_secs(2)))?;
        let cert = tls
            .ssl()
            .peer_certificate()
            .ok_or(NativeError::InvalidFrame)?;
        validate_peer_certificate(&cert)?;
        let peer_fp = fingerprint(&cert)?;
        let hello = read_frame(&mut tls)?;
        let Message::Hello {
            protocol: WIRE_VERSION,
            id,
            name,
            trusted_server_id,
            pair_commit,
        } = hello
        else {
            return Err(NativeError::InvalidFrame);
        };
        if id != peer_fp || name.is_empty() || name.len() > 128 {
            return Err(NativeError::InvalidFrame);
        }
        // Every ceremony gets a fresh nonce. The hello carries only its
        // commitment; the opening follows, so a middlebox that committed to
        // its certificates at the TLS handshake cannot grind the displayed
        // code offline before the users compare it.
        let mut nonce = [0u8; 16];
        openssl::rand::rand_bytes(&mut nonce)?;
        let nonce_hex = hex::encode(nonce);
        write_frame(
            &mut tls,
            &Message::Hello {
                protocol: WIRE_VERSION,
                id: self.id.clone(),
                name: "Linux desktop".into(),
                trusted_server_id: None,
                pair_commit: Some(hex::encode(Sha256::digest(nonce))),
            },
        )?;
        let peer = Peer {
            id: id.clone(),
            name: name.clone(),
            fingerprint: peer_fp.clone(),
        };
        if trusted_server_id.as_deref() == Some(self.id.as_str())
            && !self.inner.lock().unwrap().peers.peers.contains_key(&id)
        {
            write_frame(
                &mut tls,
                &Message::Revoke {
                    protocol: WIRE_VERSION,
                },
            )?;
            return Ok(());
        }
        let known = {
            let mut inner = self.inner.lock().unwrap();
            if inner.active.contains_key(&id) || !inner.connecting.insert(id.clone()) {
                // A live session for this peer must not be overwritten by a
                // second concurrent connection with the same identity.
                return Err(NativeError::InvalidFrame);
            }
            let known = inner
                .peers
                .peers
                .get(&id)
                .is_some_and(|p| p.fingerprint == peer_fp);
            if !known {
                inner
                    .pending
                    .retain(|_, candidate| candidate.created.elapsed() < Duration::from_secs(120));
                if inner.pending.len() >= MAX_SESSIONS {
                    inner.connecting.remove(&id);
                    return Err(NativeError::InvalidFrame);
                }
                let Some(commit) = pair_commit else {
                    // Unknown peers must commit to a fresh nonce; without it
                    // the ceremony code would be a static function of the
                    // certificates and grindable offline by a middlebox.
                    inner.connecting.remove(&id);
                    return Err(NativeError::InvalidFrame);
                };
                if !hex::decode(&commit).is_ok_and(|bytes| bytes.len() == 32) {
                    inner.connecting.remove(&id);
                    return Err(NativeError::InvalidFrame);
                }
                inner.pending.insert(
                    id.clone(),
                    Candidate {
                        peer: peer.clone(),
                        // Computed once the phone reveals its nonce.
                        code: String::new(),
                        commit,
                        approved: false,
                        created: Instant::now(),
                    },
                );
            }
            known
        };
        let mut session = Session {
            backend: self,
            peer: peer.clone(),
            event,
            published: false,
        };
        if !known {
            // Created after the session guard so a failed opening write
            // still releases the identity slot for a fresh ceremony.
            write_frame(
                &mut tls,
                &Message::PairOpen {
                    protocol: WIRE_VERSION,
                    nonce: nonce_hex.clone(),
                },
            )?;
        }
        if !known {
            // Both sides committed to a fresh nonce in their hellos and have
            // now revealed the openings. Each side verifies the peer's
            // opening against its commitment, then both users compare the
            // resulting code out of band and approve on their own side: Linux
            // through local IPC, the phone by repeating the code it
            // displayed. Pairing completes only when the local approval and
            // the matching phone confirmation meet within one ceremony.
            let started = Instant::now();
            let mut opened = false;
            let mut phone_confirmed = false;
            loop {
                if started.elapsed() > Duration::from_secs(120) {
                    return Err(NativeError::InvalidFrame);
                }
                match read_frame(&mut tls) {
                    Ok(Message::PairOpen {
                        protocol: WIRE_VERSION,
                        nonce: peer_nonce,
                    }) => {
                        if opened {
                            return Err(NativeError::InvalidFrame);
                        }
                        let Some(bytes) = parse_nonce(&peer_nonce) else {
                            return Err(NativeError::InvalidFrame);
                        };
                        let mut inner = self.inner.lock().unwrap();
                        let Some(candidate) = inner.pending.get_mut(&id) else {
                            return Err(NativeError::InvalidFrame);
                        };
                        if hex::encode(Sha256::digest(bytes)) != candidate.commit {
                            // The opening does not match the hello's
                            // commitment: drop the ceremony instead of
                            // displaying a code the peer did not commit to.
                            return Err(NativeError::InvalidFrame);
                        }
                        candidate.code =
                            comparison_code(&self.id, &nonce_hex, &peer_fp, &peer_nonce);
                        opened = true;
                    }
                    Ok(Message::PairConfirm {
                        protocol: WIRE_VERSION,
                        code,
                    }) => {
                        let expected = self
                            .inner
                            .lock()
                            .unwrap()
                            .pending
                            .get(&id)
                            .map_or(String::new(), |candidate| candidate.code.clone());
                        if expected.is_empty() || code.as_deref() != Some(expected.as_str()) {
                            // A confirmation that does not repeat the displayed
                            // ceremony code is a malfunction or an attack. Drop
                            // the session so any retry starts a fresh,
                            // user-visible ceremony instead of allowing
                            // unlimited guesses against this one.
                            return Err(NativeError::InvalidFrame);
                        }
                        phone_confirmed = true;
                    }
                    Ok(Message::Ping {
                        protocol: WIRE_VERSION,
                    }) => write_frame(
                        &mut tls,
                        &Message::Pong {
                            protocol: WIRE_VERSION,
                        },
                    )?,
                    Err(NativeError::Io(e))
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut => {}
                    _ => return Err(NativeError::InvalidFrame),
                }
                let mut inner = self.inner.lock().unwrap();
                if phone_confirmed && inner.pending.get(&id).is_some_and(|p| p.approved) {
                    let mut peers = inner.peers.clone();
                    peers.peers.insert(id.clone(), peer.clone());
                    self.save_peers(&peers)?;
                    inner.peers = peers;
                    inner.pending.remove(&id);
                    break;
                }
            }
        }
        write_frame(
            &mut tls,
            &Message::Paired {
                protocol: WIRE_VERSION,
            },
        )?;
        let added = {
            let mut inner = self.inner.lock().unwrap();
            if !inner.peers.peers.contains_key(&id) {
                return Err(NativeError::InvalidFrame);
            }
            inner.active.insert(id.clone(), tls.get_ref().try_clone()?);
            session.published = true;
            device(
                &peer,
                true,
                inner.batteries.get(&id).cloned(),
                inner.notif_enabled.get(&id).copied().unwrap_or(false),
                inner.media_enabled.get(&id).copied().unwrap_or(false),
            )
        };
        // The device event emits without the lock held: callbacks may call
        // back into the backend.
        event(StateEvent::Device(DeviceEvent::Added(added)));
        // Ask a freshly paired phone for its current notification list. The
        // phone also syncs proactively after `paired`; the request covers a
        // daemon restart where the phone never saw the pairing transition.
        let _ = write_frame(
            &mut tls,
            &Message::NotificationsRequest {
                protocol: WIRE_VERSION,
            },
        );
        let _ = write_frame(
            &mut tls,
            &Message::CallRequest {
                protocol: WIRE_VERSION,
            },
        );
        // Same recovery for media sessions: a daemon restart must not wait
        // for the next playback change to learn the current players.
        let _ = write_frame(
            &mut tls,
            &Message::MediaRequest {
                protocol: WIRE_VERSION,
            },
        );
        let mut last_received = Instant::now();
        let session_started = Instant::now();
        let mut snapshot_retry_sent = false;
        loop {
            if last_received.elapsed() > Duration::from_secs(90) {
                break;
            }
            if !self.inner.lock().unwrap().peers.peers.contains_key(&id) {
                break;
            }
            if !snapshot_retry_sent && session_started.elapsed() >= Duration::from_secs(5) {
                let notification_answered = {
                    let inner = self.inner.lock().unwrap();
                    inner.notif_enabled.contains_key(&id)
                };
                if !notification_answered {
                    let _ = write_frame(
                        &mut tls,
                        &Message::NotificationsRequest {
                            protocol: WIRE_VERSION,
                        },
                    );
                }
                snapshot_retry_sent = true;
            }
            let expired = {
                let mut inner = self.inner.lock().unwrap();
                let pending = inner.pending_shares.entry(id.clone()).or_default();
                let expired = take_expired_shares(pending, Instant::now());
                if let Some(queue) = inner.outbox.get_mut(&id) {
                    let mut temps = Vec::new();
                    queue.retain(|message| {
                        let drop_it = match message {
                            Message::ShareUrl { transfer_id, .. }
                            | Message::ShareFile { transfer_id, .. } => {
                                expired.contains(transfer_id)
                            }
                            _ => false,
                        };
                        if drop_it {
                            temps.extend(clipboard_temp_path(message).cloned());
                        }
                        !drop_it
                    });
                    drop(inner);
                    for temp in temps {
                        let _ = fs::remove_file(temp);
                    }
                }
                expired
            };
            for transfer_id in expired {
                event(StateEvent::ShareResult(ShareResult {
                    device_id: DeviceId::new(format!("native:{id}")),
                    transfer_id,
                    status: ShareStatus::Failed,
                    reason: Some(ShareFailure::TimedOut),
                }));
            }
            // Drain queued Linux-to-phone notification commands before
            // blocking on the next inbound frame. Acceptance was already
            // reported over IPC; a write failure ends the session and the
            // phone resyncs on reconnect.
            let outbound = self
                .inner
                .lock()
                .unwrap()
                .outbox
                .remove(&id)
                .unwrap_or_default();
            // A call command queued before the phone's latest report is stale:
            // state re-observation proves a call may have started or ended
            // since the user acted. Dropping it here keeps a delayed command
            // from answering, declining or hanging up a *later* call.
            let latest_generation = self
                .inner
                .lock()
                .unwrap()
                .call_generations
                .get(&id)
                .copied();
            let outbound: Vec<_> = outbound
                .into_iter()
                .filter(|message| match message {
                    Message::CallControl {
                        generation: Some(stamped),
                        ..
                    } => latest_generation == Some(*stamped),
                    _ => true,
                })
                .collect();
            for (index, message) in outbound.iter().enumerate() {
                let share_id = match message {
                    Message::ShareUrl { transfer_id, .. }
                    | Message::ShareFile { transfer_id, .. } => Some(transfer_id),
                    _ => None,
                };
                let started = share_id.and_then(|transfer_id| {
                    self.inner
                        .lock()
                        .unwrap()
                        .pending_shares
                        .get(&id)
                        .and_then(|pending| pending.get(transfer_id))
                        .copied()
                });
                if share_id.is_some() && started.is_none() {
                    continue;
                }
                if let (Some(transfer_id), Some(started)) = (share_id, started)
                    && started.elapsed() >= SHARE_RESULT_TIMEOUT
                {
                    self.inner
                        .lock()
                        .unwrap()
                        .pending_shares
                        .entry(id.clone())
                        .or_default()
                        .remove(transfer_id);
                    delete_clipboard_temp(message);
                    event(StateEvent::ShareResult(ShareResult {
                        device_id: DeviceId::new(format!("native:{id}")),
                        transfer_id: transfer_id.clone(),
                        status: ShareStatus::Failed,
                        reason: Some(ShareFailure::TimedOut),
                    }));
                    continue;
                }
                if let Err(error) = write_frame(&mut tls, message) {
                    tracing::debug!(peer = %id, %error, "native notification command write failed");
                    // The session is over; every unsent queued entry
                    // ends here. Delete their clipboard temps.
                    for remaining in outbound.iter().skip(index) {
                        delete_clipboard_temp(remaining);
                    }
                    return Err(error);
                }
                if let Message::CallControl {
                    request_id, action, ..
                } = message
                    && let Some(action) = match action.as_str() {
                        "place" => Some(CallAction::Place),
                        "answer" => Some(CallAction::Answer),
                        "decline" => Some(CallAction::Decline),
                        "hangup" => Some(CallAction::Hangup),
                        _ => None,
                    }
                {
                    self.inner
                        .lock()
                        .unwrap()
                        .pending_call_results
                        .entry(id.clone())
                        .or_default()
                        .insert(request_id.clone(), action);
                }
                if let Message::ShareFile {
                    path,
                    size,
                    transfer_id,
                    clipboard,
                    ..
                } = message
                {
                    // A send that never completes must not leave its
                    // clipboard temp behind. User files are never
                    // touched; only clipboard-owned temps are cleaned.
                    let temp = clipboard.then(|| path.clone());
                    let cleanup = |temp: &Option<PathBuf>| {
                        if let Some(path) = temp {
                            let _ = fs::remove_file(path);
                        }
                    };
                    let file = File::open(path).inspect_err(|_| cleanup(&temp))?;
                    let metadata = file.metadata().inspect_err(|_| cleanup(&temp))?;
                    if !metadata.is_file() || metadata.len() != *size {
                        cleanup(&temp);
                        return Err(NativeError::InvalidFrame);
                    }
                    let started = started.ok_or(NativeError::InvalidFrame).inspect_err(|_| {
                        cleanup(&temp);
                    })?;
                    let device_id = DeviceId::new(format!("native:{id}"));
                    let mut report_progress = |bytes_sent| {
                        event(StateEvent::ShareProgress(ShareProgress {
                            device_id: device_id.clone(),
                            transfer_id: transfer_id.clone(),
                            bytes_sent,
                            total_bytes: *size,
                        }));
                    };
                    if let Err(error) = stream_file_until(
                        &mut file.take(*size),
                        &mut tls,
                        *size,
                        started,
                        &mut report_progress,
                    ) {
                        if error.kind() == std::io::ErrorKind::TimedOut {
                            self.inner
                                .lock()
                                .unwrap()
                                .pending_shares
                                .entry(id.clone())
                                .or_default()
                                .remove(transfer_id);
                            event(StateEvent::ShareResult(ShareResult {
                                device_id: DeviceId::new(format!("native:{id}")),
                                transfer_id: transfer_id.clone(),
                                status: ShareStatus::Failed,
                                reason: Some(ShareFailure::TimedOut),
                            }));
                        }
                        cleanup(&temp);
                        return Err(NativeError::Io(error));
                    }
                    if let Err(error) = tls.flush() {
                        cleanup(&temp);
                        return Err(NativeError::Io(error));
                    }
                    if *clipboard {
                        let _ = fs::remove_file(path);
                    }
                }
            }
            match read_frame(&mut tls) {
                Ok(Message::ShareUrl {
                    protocol: WIRE_VERSION,
                    transfer_id,
                    url,
                }) => {
                    last_received = Instant::now();
                    if !valid_transfer_id(&transfer_id) || !valid_share_url(&url) {
                        write_frame(
                            &mut tls,
                            &Message::ShareResult {
                                protocol: WIRE_VERSION,
                                transfer_id,
                                status: ShareStatus::Failed,
                                reason: Some(ShareFailure::InvalidResource),
                            },
                        )?;
                        continue;
                    }
                    event(StateEvent::ShareReceived(ReceivedShare {
                        device_id: DeviceId::new(format!("native:{id}")),
                        resource: SharedResource::Url { url },
                    }));
                    write_frame(
                        &mut tls,
                        &Message::ShareResult {
                            protocol: WIRE_VERSION,
                            transfer_id,
                            status: ShareStatus::Completed,
                            reason: None,
                        },
                    )?;
                }
                Ok(Message::ShareFile {
                    protocol: WIRE_VERSION,
                    transfer_id,
                    name,
                    size,
                    ..
                }) => {
                    let rejected = if !valid_transfer_id(&transfer_id)
                        || !safe_share_name(&name)
                        || name.len() > 255
                    {
                        Some(ShareFailure::InvalidResource)
                    } else if size > MAX_SHARE_SIZE {
                        Some(ShareFailure::SizeLimit)
                    } else {
                        None
                    };
                    if let Some(reason) = rejected {
                        let _ = write_frame(
                            &mut tls,
                            &Message::ShareResult {
                                protocol: WIRE_VERSION,
                                transfer_id,
                                status: ShareStatus::Failed,
                                reason: Some(reason),
                            },
                        );
                        return Err(NativeError::InvalidFrame);
                    }
                    let path = match self.receive_share_file(&mut tls, &name, size) {
                        Ok(path) => path,
                        Err(error) => {
                            let reason = match &error {
                                NativeError::Io(io)
                                    if io.kind() == std::io::ErrorKind::UnexpectedEof =>
                                {
                                    ShareFailure::Interrupted
                                }
                                _ => ShareFailure::Storage,
                            };
                            let _ = write_frame(
                                &mut tls,
                                &Message::ShareResult {
                                    protocol: WIRE_VERSION,
                                    transfer_id,
                                    status: ShareStatus::Failed,
                                    reason: Some(reason),
                                },
                            );
                            return Err(error);
                        }
                    };
                    last_received = Instant::now();
                    event(StateEvent::ShareReceived(ReceivedShare {
                        device_id: DeviceId::new(format!("native:{id}")),
                        resource: SharedResource::File {
                            path: path.to_string_lossy().into_owned(),
                        },
                    }));
                    write_frame(
                        &mut tls,
                        &Message::ShareResult {
                            protocol: WIRE_VERSION,
                            transfer_id,
                            status: ShareStatus::Completed,
                            reason: None,
                        },
                    )?;
                }
                Ok(Message::ShareResult {
                    protocol: WIRE_VERSION,
                    transfer_id,
                    status,
                    reason,
                }) => {
                    last_received = Instant::now();
                    if !valid_transfer_id(&transfer_id)
                        || (status == ShareStatus::Completed && reason.is_some())
                        || (status == ShareStatus::Failed && reason.is_none())
                    {
                        return Err(NativeError::InvalidFrame);
                    }
                    let was_pending = self
                        .inner
                        .lock()
                        .unwrap()
                        .pending_shares
                        .entry(id.clone())
                        .or_default()
                        .remove(&transfer_id)
                        .is_some();
                    if was_pending {
                        event(StateEvent::ShareResult(ShareResult {
                            device_id: DeviceId::new(format!("native:{id}")),
                            transfer_id,
                            status,
                            reason,
                        }));
                    }
                }
                Ok(Message::DeviceCommandResult {
                    protocol: WIRE_VERSION,
                    request_id,
                    action,
                    accepted,
                    failure,
                }) => {
                    last_received = Instant::now();
                    if !valid_transfer_id(&request_id)
                        || (accepted && failure.is_some())
                        || (!accepted && failure.is_none())
                    {
                        return Err(NativeError::InvalidFrame);
                    }
                    event(StateEvent::DeviceCommandResult(DeviceCommandResult {
                        device_id: DeviceId::new(format!("native:{id}")),
                        request_id,
                        action,
                        accepted,
                        failure,
                    }));
                }
                Ok(Message::ScreensaverControl {
                    protocol: WIRE_VERSION,
                    request_id,
                    inhibit,
                }) => {
                    last_received = Instant::now();
                    if !valid_transfer_id(&request_id) {
                        return Err(NativeError::InvalidFrame);
                    }
                    // The phone only requests; the daemon owns the inhibitor.
                    // A release from a peer that never requested is still
                    // accepted so both sides converge on awake policy.
                    let mut inner = self.inner.lock().unwrap();
                    if inhibit {
                        inner.screensaver_requests.insert(id.clone());
                    } else {
                        inner.screensaver_requests.remove(&id);
                    }
                    drop(inner);
                    event(StateEvent::DeviceCommandResult(DeviceCommandResult {
                        device_id: DeviceId::new(format!("native:{id}")),
                        request_id,
                        action: DeviceCommandAction::Screensaver,
                        accepted: true,
                        failure: None,
                    }));
                }
                Ok(Message::Battery {
                    protocol: WIRE_VERSION,
                    percentage,
                    charging,
                }) => {
                    last_received = Instant::now();
                    let battery = BatteryState::new(percentage, charging)
                        .map_err(|_| NativeError::InvalidFrame)?;
                    let updated = {
                        let mut inner = self.inner.lock().unwrap();
                        if !inner.peers.peers.contains_key(&id) {
                            break;
                        }
                        inner.batteries.insert(id.clone(), battery);
                        let notifications_supported =
                            inner.notif_enabled.get(&id).copied().unwrap_or(false);
                        let media_supported =
                            inner.media_enabled.get(&id).copied().unwrap_or(false);
                        let mut updated = device(
                            &peer,
                            true,
                            Some(battery),
                            notifications_supported,
                            media_supported,
                        );
                        updated.connectivity = inner.connectivity.get(&id).copied();
                        if updated.connectivity.is_some() {
                            updated.capabilities.insert(Capability::Connectivity);
                        }
                        updated
                    };
                    // Emit without the lock held: callbacks may call back
                    // into the backend.
                    event(StateEvent::Device(DeviceEvent::Updated(updated)));
                }
                Ok(Message::Connectivity {
                    protocol: WIRE_VERSION,
                    transport,
                    validated,
                    metered,
                }) => {
                    last_received = Instant::now();
                    let connectivity = ConnectivityState {
                        transport,
                        validated,
                        metered,
                    };
                    let updated = {
                        let mut inner = self.inner.lock().unwrap();
                        if !inner.peers.peers.contains_key(&id) {
                            break;
                        }
                        inner.connectivity.insert(id.clone(), connectivity);
                        let mut updated = device(
                            &peer,
                            true,
                            inner.batteries.get(&id).cloned(),
                            inner.notif_enabled.get(&id).copied().unwrap_or(false),
                            inner.media_enabled.get(&id).copied().unwrap_or(false),
                        );
                        updated.connectivity = Some(connectivity);
                        updated.capabilities.insert(Capability::Connectivity);
                        updated
                    };
                    // Emit without the lock held: callbacks may call back
                    // into the backend.
                    event(StateEvent::Device(DeviceEvent::Updated(updated)));
                }
                Ok(Message::CallState {
                    protocol: WIRE_VERSION,
                    phase,
                    controls,
                    generation,
                }) => {
                    last_received = Instant::now();
                    // The generation is the phone's own counter, not a daemon
                    // guess: storing the received value keeps stamps and the
                    // phone's execution check in one sequence.
                    self.inner
                        .lock()
                        .unwrap()
                        .call_generations
                        .insert(id.clone(), generation);
                    event(StateEvent::Call(CallEvent::Updated(CallState {
                        device_id: DeviceId::new(format!("native:{id}")),
                        phase,
                        controls,
                        generation,
                    })));
                }
                Ok(Message::CallResult {
                    protocol: WIRE_VERSION,
                    request_id,
                    action,
                    accepted,
                    failure,
                }) => {
                    last_received = Instant::now();
                    let pending_action = self
                        .inner
                        .lock()
                        .unwrap()
                        .pending_call_results
                        .entry(id.clone())
                        .or_default()
                        .remove(&request_id);
                    if !valid_transfer_id(&request_id)
                        || pending_action != Some(action)
                        || (accepted && failure.is_some())
                        || (!accepted && failure.is_none())
                    {
                        return Err(NativeError::InvalidFrame);
                    }
                    event(StateEvent::CallCommandResult(CallCommandResult {
                        device_id: DeviceId::new(format!("native:{id}")),
                        request_id,
                        action,
                        accepted,
                        failure,
                    }));
                }
                Ok(Message::PresentationControl {
                    protocol: WIRE_VERSION,
                    action,
                    delta_x,
                    delta_y,
                }) => {
                    last_received = Instant::now();
                    event(StateEvent::Presentation(PresentationCommand {
                        device_id: DeviceId::new(format!("native:{id}")),
                        action,
                        delta_x,
                        delta_y,
                    }));
                }
                Ok(Message::VolumeControl {
                    protocol: WIRE_VERSION,
                    action,
                }) => {
                    last_received = Instant::now();
                    event(StateEvent::Volume(VolumeCommand {
                        device_id: DeviceId::new(format!("native:{id}")),
                        action,
                    }));
                }
                Ok(Message::RemoteInputControl {
                    protocol: WIRE_VERSION,
                    action,
                    delta_x,
                    delta_y,
                    button,
                    text,
                }) => {
                    let valid = match action {
                        RemoteInputAction::Move => delta_x.abs() <= 2000 && delta_y.abs() <= 2000,
                        RemoteInputAction::Click => (1..=5).contains(&button),
                        RemoteInputAction::Scroll => delta_y.unsigned_abs() <= 20,
                        RemoteInputAction::Type => {
                            text.as_ref().is_some_and(|value| value.len() <= 512)
                        }
                    };
                    if !valid {
                        return Err(NativeError::InvalidFrame);
                    }
                    last_received = Instant::now();
                    event(StateEvent::RemoteInput(RemoteInputCommand {
                        device_id: DeviceId::new(format!("native:{id}")),
                        action,
                        delta_x,
                        delta_y,
                        button,
                        text,
                    }));
                }
                Ok(Message::ContactsSync {
                    protocol: WIRE_VERSION,
                    contacts,
                }) => {
                    last_received = Instant::now();
                    let device_id = DeviceId::new(format!("native:{id}"));
                    let contacts = contacts
                        .into_iter()
                        .map(|contact| Contact {
                            device_id: device_id.clone(),
                            local_id: contact.local_id,
                            display_name: contact.display_name,
                            phones: contact.phones,
                            emails: contact.emails,
                            photo: contact.photo,
                        })
                        .collect();
                    event(StateEvent::Contacts(ContactsEvent::Synced {
                        device_id,
                        contacts,
                    }));
                }
                Ok(Message::ClipboardPost {
                    protocol: WIRE_VERSION,
                    text,
                    html,
                    uri,
                }) => {
                    last_received = Instant::now();
                    let rich_size = text.len()
                        + html.as_ref().map_or(0, String::len)
                        + uri.as_ref().map_or(0, String::len);
                    if text.len() <= 32 * 1024
                        && html.as_ref().is_none_or(|value| value.len() <= 32 * 1024)
                        && uri.as_ref().is_none_or(|value| value.len() <= 32 * 1024)
                        && rich_size <= 48 * 1024
                    {
                        event(StateEvent::Clipboard(ClipboardText {
                            device_id: DeviceId::new(format!("native:{id}")),
                            text,
                            html,
                            uri,
                        }));
                    }
                }
                Ok(Message::ClipboardFile {
                    protocol: WIRE_VERSION,
                    transfer_id,
                    name,
                    size,
                    mime,
                }) => {
                    if !valid_transfer_id(&transfer_id)
                        || size > 10 * 1024 * 1024
                        || !safe_share_name(&name)
                        || name.len() > 255
                        || mime.is_empty()
                        || mime.len() > 128
                    {
                        let _ = write_frame(
                            &mut tls,
                            &Message::ClipboardResult {
                                protocol: WIRE_VERSION,
                                transfer_id,
                                status: ShareStatus::Failed,
                                reason: Some(ShareFailure::InvalidResource),
                            },
                        );
                        return Err(NativeError::InvalidFrame);
                    }
                    let path = match self.receive_share_file(&mut tls, &name, size) {
                        Ok(path) => path,
                        Err(error) => {
                            let reason = match &error {
                                NativeError::Io(io)
                                    if io.kind() == std::io::ErrorKind::UnexpectedEof =>
                                {
                                    ShareFailure::Interrupted
                                }
                                _ => ShareFailure::Storage,
                            };
                            let _ = write_frame(
                                &mut tls,
                                &Message::ClipboardResult {
                                    protocol: WIRE_VERSION,
                                    transfer_id,
                                    status: ShareStatus::Failed,
                                    reason: Some(reason),
                                },
                            );
                            return Err(error);
                        }
                    };
                    event(StateEvent::ClipboardFile(ClipboardFile {
                        device_id: DeviceId::new(format!("native:{id}")),
                        path: path.to_string_lossy().into_owned(),
                        mime,
                    }));
                    last_received = Instant::now();
                    write_frame(
                        &mut tls,
                        &Message::ClipboardResult {
                            protocol: WIRE_VERSION,
                            transfer_id,
                            status: ShareStatus::Completed,
                            reason: None,
                        },
                    )?;
                }
                Ok(Message::NotificationPost {
                    protocol: WIRE_VERSION,
                    key,
                    app,
                    title,
                    body,
                    clearable,
                    actions,
                    reply_supported,
                }) => {
                    last_received = Instant::now();
                    self.handle_notification_post(
                        &peer,
                        WireNotification {
                            key,
                            app,
                            title,
                            body,
                            clearable,
                            actions,
                            reply_supported,
                        },
                        event,
                    )?;
                }
                Ok(Message::NotificationRemoved {
                    protocol: WIRE_VERSION,
                    key,
                }) => {
                    last_received = Instant::now();
                    self.handle_notification_removed(&peer, &key, event)?;
                }
                Ok(Message::NotificationsSync {
                    protocol: WIRE_VERSION,
                    enabled,
                    notifications,
                }) => {
                    last_received = Instant::now();
                    self.handle_notifications_sync(&peer, enabled, notifications, event)?;
                }
                Ok(Message::MediaPost {
                    protocol: WIRE_VERSION,
                    player,
                    application,
                    title,
                    artist,
                    album,
                    playback,
                    position_ms,
                    duration_ms,
                    controls,
                }) => {
                    last_received = Instant::now();
                    self.handle_media_post(
                        &peer,
                        WireMediaSession {
                            player,
                            application,
                            title,
                            artist,
                            album,
                            playback,
                            position_ms,
                            duration_ms,
                            controls,
                        },
                        event,
                    )?;
                }
                Ok(Message::MediaRemoved {
                    protocol: WIRE_VERSION,
                    player,
                }) => {
                    last_received = Instant::now();
                    self.handle_media_removed(&peer, &player, event)?;
                }
                Ok(Message::MediaSync {
                    protocol: WIRE_VERSION,
                    sessions,
                }) => {
                    last_received = Instant::now();
                    self.handle_media_sync(&peer, sessions, event)?;
                }
                Ok(Message::Ping {
                    protocol: WIRE_VERSION,
                }) => {
                    last_received = Instant::now();
                    write_frame(
                        &mut tls,
                        &Message::Pong {
                            protocol: WIRE_VERSION,
                        },
                    )?;
                }
                Ok(Message::Revoke {
                    protocol: WIRE_VERSION,
                }) => {
                    self.unpair(&id)?;
                    break;
                }
                Err(NativeError::Io(e))
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}
                _ => break,
            }
        }
        Ok(())
    }
}

struct PartialShare(PathBuf, PathBuf);

impl Drop for PartialShare {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.0);
            let _ = fs::remove_dir(&self.1);
        }
    }
}

fn safe_share_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

fn new_transfer_id() -> Result<String, openssl::error::ErrorStack> {
    let mut bytes = [0u8; 16];
    openssl::rand::rand_bytes(&mut bytes)?;
    Ok(hex::encode(bytes))
}

fn valid_transfer_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn take_expired_shares(pending: &mut BTreeMap<String, Instant>, now: Instant) -> Vec<String> {
    let expired = pending
        .iter()
        .filter(|(_, started)| now.duration_since(**started) >= SHARE_RESULT_TIMEOUT)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for id in &expired {
        pending.remove(id);
    }
    expired
}

fn stream_file_until<R: Read, W: Write>(
    input: &mut R,
    output: &mut W,
    size: u64,
    started: Instant,
    progress: &mut impl FnMut(u64),
) -> std::io::Result<()> {
    let mut remaining = size;
    let mut next_progress = 256 * 1024;
    let mut buffer = [0u8; SHARE_BUFFER];
    progress(0);
    while remaining > 0 {
        if started.elapsed() >= SHARE_RESULT_TIMEOUT {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "share result deadline",
            ));
        }
        let amount = usize::try_from(remaining.min(SHARE_BUFFER as u64)).unwrap();
        input.read_exact(&mut buffer[..amount])?;
        output.write_all(&buffer[..amount])?;
        remaining -= amount as u64;
        let sent = size - remaining;
        if sent >= next_progress || remaining == 0 {
            progress(sent);
            next_progress = sent.saturating_add(256 * 1024);
        }
    }
    output.flush()
}

fn read_exact_until<R: Read>(
    input: &mut R,
    buffer: &mut [u8],
    deadline: Instant,
) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < buffer.len() {
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "inbound transfer deadline",
            ));
        }
        match input.read(&mut buffer[offset..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "inbound transfer ended early",
                ));
            }
            Ok(amount) => offset += amount,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "inbound transfer deadline",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn valid_share_url(url: &str) -> bool {
    if url.is_empty()
        || url.len() > 8192
        || url.trim() != url
        || url.chars().any(char::is_whitespace)
        || url.chars().any(char::is_control)
    {
        return false;
    }
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    !matches!(parsed.scheme(), "file" | "javascript" | "data")
}

fn device(
    peer: &Peer,
    connected: bool,
    battery: Option<BatteryState>,
    notifications_supported: bool,
    media_supported: bool,
) -> Device {
    let mut capabilities = BTreeSet::from([Capability::Battery, Capability::FileTransfer]);
    if notifications_supported {
        capabilities.insert(Capability::Notifications);
    }
    if media_supported {
        capabilities.insert(Capability::Media);
    }
    Device {
        id: DeviceId::new(format!("native:{}", peer.id)),
        name: peer.name.clone(),
        connected,
        paired: true,
        battery,
        connectivity: None,
        capabilities,
    }
}

/// Validates one phone-reported notification without logging its content.
/// Titles and bodies never reach normal log levels; callers log only the
/// device-scoped key and counts.
fn normalize_native_notification(
    peer: &Peer,
    wire: &WireNotification,
) -> Result<Notification, NativeError> {
    let key = wire.key.trim();
    if key.is_empty() || key.len() > MAX_NOTIFICATION_KEY || key.chars().any(char::is_control) {
        return Err(NativeError::InvalidFrame);
    }
    if wire.app.len() > MAX_NOTIFICATION_APP
        || wire.title.len() > MAX_NOTIFICATION_TITLE
        || wire.body.len() > MAX_NOTIFICATION_BODY
    {
        return Err(NativeError::InvalidFrame);
    }
    if wire.actions.len() > MAX_NOTIFICATION_ACTIONS {
        return Err(NativeError::InvalidFrame);
    }
    let mut seen = BTreeSet::new();
    let mut actions = Vec::with_capacity(wire.actions.len());
    for action in &wire.actions {
        if action.id.is_empty()
            || action.id.len() > MAX_NOTIFICATION_ACTION_ID
            || action.label.is_empty()
            || action.label.len() > MAX_NOTIFICATION_ACTION_LABEL
            || !seen.insert(action.id.clone())
        {
            return Err(NativeError::InvalidFrame);
        }
        actions.push(NotificationAction {
            id: action.id.clone(),
            label: action.label.clone(),
        });
    }
    Ok(Notification {
        id: NotificationId::new(DeviceId::new(format!("native:{}", peer.id)), key.to_owned()),
        app_name: wire.app.clone(),
        title: wire.title.clone(),
        body: wire.body.clone(),
        icon_path: None,
        clearable: wire.clearable,
        actions,
        reply_supported: wire.reply_supported,
    })
}

/// Validates one phone-reported media session without logging its content.
/// Track titles and artists never reach normal log levels; callers log only
/// the device-scoped player id and counts.
fn normalize_native_media(
    peer: &Peer,
    wire: &WireMediaSession,
) -> Result<MediaSession, NativeError> {
    let player = wire.player.trim();
    if player.is_empty() || player.len() > MAX_MEDIA_PLAYER || player.chars().any(char::is_control)
    {
        return Err(NativeError::InvalidFrame);
    }
    if wire.application.len() > MAX_MEDIA_APP {
        return Err(NativeError::InvalidFrame);
    }
    for text in [&wire.title, &wire.artist, &wire.album]
        .into_iter()
        .flatten()
    {
        if text.len() > MAX_MEDIA_TEXT {
            return Err(NativeError::InvalidFrame);
        }
    }
    for bound in [wire.position_ms, wire.duration_ms].into_iter().flatten() {
        if bound > MAX_MEDIA_POSITION_MS {
            return Err(NativeError::InvalidFrame);
        }
    }
    let mut controls = BTreeSet::new();
    for control in &wire.controls {
        match control {
            WireControl::Play => controls.insert(MediaControl::Play),
            WireControl::Pause => controls.insert(MediaControl::Pause),
            WireControl::PlayPause => controls.insert(MediaControl::PlayPause),
            WireControl::Next => controls.insert(MediaControl::Next),
            WireControl::Previous => controls.insert(MediaControl::Previous),
            WireControl::SetPosition => controls.insert(MediaControl::SetPosition),
            // Relative seeks have no genuine Android API behind them; a
            // phone advertising one is malfunctioning or malicious.
            WireControl::Seek => return Err(NativeError::InvalidFrame),
        };
    }
    let playback = match wire.playback {
        WirePlayback::Playing => PlaybackState::Playing,
        WirePlayback::Paused => PlaybackState::Paused,
        WirePlayback::Stopped => PlaybackState::Stopped,
        WirePlayback::Unknown => PlaybackState::Unknown,
    };
    Ok(MediaSession {
        id: MediaSessionId::new(
            DeviceId::new(format!("native:{}", peer.id)),
            player.to_owned(),
        ),
        application: wire.application.clone(),
        title: wire.title.clone().filter(|text| !text.is_empty()),
        artist: wire.artist.clone().filter(|text| !text.is_empty()),
        album: wire.album.clone().filter(|text| !text.is_empty()),
        playback,
        position_ms: wire.position_ms,
        duration_ms: wire.duration_ms,
        // Volume is read-only in the Handover model and never transported.
        volume_percent: None,
        controls,
    })
}

impl NativeBackend {
    fn handle_notification_post(
        &self,
        peer: &Peer,
        wire: WireNotification,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        let notification = normalize_native_notification(peer, &wire)?;
        let mut inner = self.inner.lock().unwrap();
        if !inner.peers.peers.contains_key(&peer.id) {
            return Err(NativeError::InvalidFrame);
        }
        let keys = inner.notif_keys.entry(peer.id.clone()).or_default();
        let is_new = !keys.contains(&wire.key);
        keys.insert(wire.key.clone());
        let device_changed = inner.notif_enabled.get(&peer.id).copied() != Some(true);
        inner.notif_enabled.insert(peer.id.clone(), true);
        let device_update = device_changed.then(|| {
            device(
                peer,
                true,
                inner.batteries.get(&peer.id).cloned(),
                true,
                inner.media_enabled.get(&peer.id).copied().unwrap_or(false),
            )
        });
        drop(inner);
        if let Some(device) = device_update {
            event(StateEvent::Device(DeviceEvent::Updated(device)));
        }
        event(StateEvent::Notification(if is_new {
            NotificationEvent::Added(notification)
        } else {
            NotificationEvent::Updated(notification)
        }));
        Ok(())
    }

    fn handle_notification_removed(
        &self,
        peer: &Peer,
        key: &str,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        if key.is_empty() || key.len() > MAX_NOTIFICATION_KEY {
            return Err(NativeError::InvalidFrame);
        }
        let mut inner = self.inner.lock().unwrap();
        let known = inner
            .notif_keys
            .get_mut(&peer.id)
            .is_some_and(|keys| keys.remove(key));
        drop(inner);
        if known {
            event(StateEvent::Notification(NotificationEvent::Removed(
                NotificationId::new(DeviceId::new(format!("native:{}", peer.id)), key.to_owned()),
            )));
        }
        Ok(())
    }

    fn handle_notifications_sync(
        &self,
        peer: &Peer,
        enabled: bool,
        wires: Vec<WireNotification>,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        if wires.len() > MAX_NOTIFICATIONS_PER_SYNC {
            return Err(NativeError::InvalidFrame);
        }
        let mut notifications = Vec::with_capacity(wires.len());
        let mut keys = BTreeSet::new();
        for wire in &wires {
            if !keys.insert(wire.key.clone()) {
                return Err(NativeError::InvalidFrame);
            }
            notifications.push(normalize_native_notification(peer, wire)?);
        }
        if !enabled && !notifications.is_empty() {
            return Err(NativeError::InvalidFrame);
        }
        let mut inner = self.inner.lock().unwrap();
        if !inner.peers.peers.contains_key(&peer.id) {
            return Err(NativeError::InvalidFrame);
        }
        let previous = inner
            .notif_keys
            .insert(peer.id.clone(), keys.clone())
            .unwrap_or_default();
        let device_changed = inner.notif_enabled.get(&peer.id).copied() != Some(enabled);
        inner.notif_enabled.insert(peer.id.clone(), enabled);
        let device_update = device_changed.then(|| {
            device(
                peer,
                true,
                inner.batteries.get(&peer.id).cloned(),
                enabled,
                inner.media_enabled.get(&peer.id).copied().unwrap_or(false),
            )
        });
        drop(inner);
        if let Some(device) = device_update {
            event(StateEvent::Device(DeviceEvent::Updated(device)));
        }
        for id in previous.difference(&keys) {
            event(StateEvent::Notification(NotificationEvent::Removed(
                NotificationId::new(DeviceId::new(format!("native:{}", peer.id)), id.clone()),
            )));
        }
        for notification in notifications {
            let id = notification.id.clone();
            let is_new = !previous.contains(&id.local_id);
            event(StateEvent::Notification(if is_new {
                NotificationEvent::Added(notification)
            } else {
                NotificationEvent::Updated(notification)
            }));
        }
        if !enabled {
            // Media observation shares the notification-listener permission,
            // so a revoked listener must not leave native sessions behind.
            self.clear_native_media(peer, event);
        }
        Ok(())
    }

    fn handle_media_post(
        &self,
        peer: &Peer,
        wire: WireMediaSession,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        let session = normalize_native_media(peer, &wire)?;
        let mut inner = self.inner.lock().unwrap();
        if !inner.peers.peers.contains_key(&peer.id) {
            return Err(NativeError::InvalidFrame);
        }
        let players = inner.media_players.entry(peer.id.clone()).or_default();
        let is_new = !players.contains(&wire.player);
        players.insert(wire.player.clone());
        let device_changed = inner.media_enabled.get(&peer.id).copied() != Some(true);
        inner.media_enabled.insert(peer.id.clone(), true);
        let device_update = device_changed.then(|| {
            device(
                peer,
                true,
                inner.batteries.get(&peer.id).cloned(),
                inner.notif_enabled.get(&peer.id).copied().unwrap_or(false),
                true,
            )
        });
        drop(inner);
        if let Some(device) = device_update {
            event(StateEvent::Device(DeviceEvent::Updated(device)));
        }
        event(StateEvent::Media(if is_new {
            MediaEvent::Added(session)
        } else {
            MediaEvent::Updated(session)
        }));
        Ok(())
    }

    fn handle_media_removed(
        &self,
        peer: &Peer,
        player: &str,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        if player.is_empty() || player.len() > MAX_MEDIA_PLAYER {
            return Err(NativeError::InvalidFrame);
        }
        let mut inner = self.inner.lock().unwrap();
        let known = inner
            .media_players
            .get_mut(&peer.id)
            .is_some_and(|players| players.remove(player));
        drop(inner);
        if known {
            event(StateEvent::Media(MediaEvent::Removed(MediaSessionId::new(
                DeviceId::new(format!("native:{}", peer.id)),
                player.to_owned(),
            ))));
        }
        Ok(())
    }

    fn handle_media_sync(
        &self,
        peer: &Peer,
        wires: Vec<WireMediaSession>,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        if wires.len() > MAX_MEDIA_SESSIONS_PER_SYNC {
            return Err(NativeError::InvalidFrame);
        }
        let mut sessions = Vec::with_capacity(wires.len());
        let mut players = BTreeSet::new();
        for wire in &wires {
            if !players.insert(wire.player.clone()) {
                return Err(NativeError::InvalidFrame);
            }
            sessions.push(normalize_native_media(peer, wire)?);
        }
        let mut inner = self.inner.lock().unwrap();
        if !inner.peers.peers.contains_key(&peer.id) {
            return Err(NativeError::InvalidFrame);
        }
        let previous = inner
            .media_players
            .insert(peer.id.clone(), players.clone())
            .unwrap_or_default();
        let device_changed = inner.media_enabled.get(&peer.id).copied() != Some(true);
        inner.media_enabled.insert(peer.id.clone(), true);
        let device_update = device_changed.then(|| {
            device(
                peer,
                true,
                inner.batteries.get(&peer.id).cloned(),
                inner.notif_enabled.get(&peer.id).copied().unwrap_or(false),
                true,
            )
        });
        drop(inner);
        if let Some(device) = device_update {
            event(StateEvent::Device(DeviceEvent::Updated(device)));
        }
        for player in previous.difference(&players) {
            event(StateEvent::Media(MediaEvent::Removed(MediaSessionId::new(
                DeviceId::new(format!("native:{}", peer.id)),
                player.clone(),
            ))));
        }
        for session in sessions {
            let id = session.id.clone();
            let is_new = !previous.contains(&id.player_id);
            event(StateEvent::Media(if is_new {
                MediaEvent::Added(session)
            } else {
                MediaEvent::Updated(session)
            }));
        }
        Ok(())
    }

    /// Drops the peer's native media sessions, used when the shared
    /// notification-listener permission is revoked: the listener powers
    /// media observation too, so its sessions must not linger as ghosts.
    fn clear_native_media(&self, peer: &Peer, event: &Arc<dyn Fn(StateEvent) + Send + Sync>) {
        let mut inner = self.inner.lock().unwrap();
        let previous = inner.media_players.remove(&peer.id).unwrap_or_default();
        let was_enabled = inner.media_enabled.remove(&peer.id).unwrap_or(false);
        let device_update = (was_enabled && inner.peers.peers.contains_key(&peer.id)).then(|| {
            device(
                peer,
                true,
                inner.batteries.get(&peer.id).cloned(),
                inner.notif_enabled.get(&peer.id).copied().unwrap_or(false),
                false,
            )
        });
        drop(inner);
        if let Some(device) = device_update {
            event(StateEvent::Device(DeviceEvent::Updated(device)));
        }
        let mut removals: Vec<MediaSessionId> = previous
            .into_iter()
            .map(|player| MediaSessionId::new(DeviceId::new(format!("native:{}", peer.id)), player))
            .collect();
        removals.sort();
        for id in removals {
            event(StateEvent::Media(MediaEvent::Removed(id)));
        }
    }
}

/// Registers the `_handover._tcp.local.` advertisement for a bound port and
/// returns the daemon handle, which keeps the record alive while it is held.
fn advertise(port: u16, id: &str) -> Result<ServiceDaemon, NativeError> {
    let mdns = ServiceDaemon::new().map_err(|e| NativeError::Discovery(e.to_string()))?;
    mdns.register(discovery_service(port, id)?)
        .map_err(|e| NativeError::Discovery(e.to_string()))?;
    Ok(mdns)
}

/// Builds the DNS-SD record advertised by [`NativeBackend::run`]. Discovery
/// addresses and TXT values are untrusted hints; trust comes from the TLS
/// certificate fingerprint pinned at pairing time.
fn discovery_service(port: u16, id: &str) -> Result<ServiceInfo, NativeError> {
    let name = format!("Handover-{}", &id[..12]);
    Ok(ServiceInfo::new(
        "_handover._tcp.local.",
        &name,
        &format!("{}.local.", name.to_lowercase()),
        "",
        port,
        &[("v", "1")][..],
    )
    .map_err(|e| NativeError::Discovery(e.to_string()))?
    .enable_addr_auto())
}

fn generate_identity() -> Result<(PKey<Private>, X509), NativeError> {
    let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1)?;
    let key = PKey::from_ec_key(EcKey::generate(&group)?)?;
    let mut subject = X509NameBuilder::new()?;
    subject.append_entry_by_text("CN", "Handover Linux")?;
    let subject = subject.build();
    let mut cert = X509::builder()?;
    cert.set_version(2)?;
    let mut serial = BigNum::new()?;
    serial.rand(128, MsbOption::MAYBE_ZERO, false)?;
    let serial = serial.to_asn1_integer()?;
    cert.set_serial_number(serial.as_ref())?;
    cert.set_subject_name(&subject)?;
    cert.set_issuer_name(&subject)?;
    cert.set_pubkey(&key)?;
    let not_before = Asn1Time::days_from_now(0)?;
    let not_after = Asn1Time::days_from_now(3650)?;
    cert.set_not_before(not_before.as_ref())?;
    cert.set_not_after(not_after.as_ref())?;
    cert.sign(&key, MessageDigest::sha256())?;
    Ok((key, cert.build()))
}
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), NativeError> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn fingerprint(cert: &X509) -> Result<String, NativeError> {
    Ok(hex::encode(Sha256::digest(cert.to_der()?)))
}

fn validate_peer_certificate(cert: &X509) -> Result<(), NativeError> {
    let now = Asn1Time::days_from_now(0)?;
    if cert.not_before() > now.as_ref() || cert.not_after() < now.as_ref() {
        return Err(NativeError::InvalidFrame);
    }
    // Native identities are deliberately self-signed and authenticated by the
    // pairing ceremony/fingerprint pin. Still reject malformed or non-leaf
    // certificates before accepting them as an identity.
    let key = cert.public_key()?;
    let valid_curve =
        key.id() == Id::EC && key.ec_key()?.group().curve_name() == Some(Nid::X9_62_PRIME256V1);
    if !valid_curve || !cert.verify(&key)? {
        return Err(NativeError::InvalidFrame);
    }
    Ok(())
}
fn comparison_code(own_fp: &str, own_nonce: &str, peer_fp: &str, peer_nonce: &str) -> String {
    // Each nonce stays bound to its fingerprint owner, so both sides derive
    // the same code without roles while a middlebox cannot swap openings.
    let ((low_fp, low_nonce), (high_fp, high_nonce)) = if own_fp <= peer_fp {
        ((own_fp, own_nonce), (peer_fp, peer_nonce))
    } else {
        ((peer_fp, peer_nonce), (own_fp, own_nonce))
    };
    let digest = Sha256::digest(
        format!("handover-pair-v2:{low_fp}:{high_fp}:{low_nonce}:{high_nonce}").as_bytes(),
    );
    let number = u32::from_be_bytes(digest[..4].try_into().unwrap()) % 100_000_000;
    format!("{number:08}")
}
fn parse_nonce(hex_nonce: &str) -> Option<[u8; 16]> {
    hex::decode(hex_nonce).ok()?.try_into().ok()
}
fn read_frame<R: Read>(reader: &mut R) -> Result<Message, NativeError> {
    let mut len = [0u8; 4];
    reader.read_exact(&mut len[..1])?;
    reader
        .read_exact(&mut len[1..])
        .map_err(|_| NativeError::InvalidFrame)?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(NativeError::InvalidFrame);
    }
    let mut data = vec![0; len];
    reader
        .read_exact(&mut data)
        .map_err(|_| NativeError::InvalidFrame)?;
    let msg: Message = serde_json::from_slice(&data)?;
    if msg.version() != WIRE_VERSION {
        return Err(NativeError::InvalidFrame);
    }
    Ok(msg)
}
fn write_frame<W: Write>(writer: &mut W, message: &Message) -> Result<(), NativeError> {
    let data = serde_json::to_vec(message)?;
    if data.is_empty() || data.len() > MAX_FRAME {
        return Err(NativeError::InvalidFrame);
    }
    writer.write_all(&(data.len() as u32).to_be_bytes())?;
    writer.write_all(&data)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_command_and_permission_result_use_correlated_wire_ids() {
        let request_id = "0123456789abcdef0123456789abcdef";
        let command = Message::LockDevice {
            protocol: WIRE_VERSION,
            request_id: request_id.into(),
        };
        let result = Message::DeviceCommandResult {
            protocol: WIRE_VERSION,
            request_id: request_id.into(),
            action: DeviceCommandAction::Lock,
            accepted: false,
            failure: Some(DeviceCommandFailure::PermissionDenied),
        };

        assert_eq!(
            serde_json::to_value(command).expect("command serializes"),
            serde_json::json!({
                "type": "lock_device", "protocol": 1, "request_id": request_id
            })
        );
        assert_eq!(
            serde_json::to_value(result).expect("result serializes"),
            serde_json::json!({
                "type": "device_command_result", "protocol": 1,
                "request_id": request_id, "action": "lock", "accepted": false,
                "failure": "permission_denied"
            })
        );
    }

    #[test]
    fn rich_clipboard_command_carries_result_correlation_id() {
        let request_id = "0123456789abcdef0123456789abcdef";
        let command = Message::ClipboardSet {
            protocol: WIRE_VERSION,
            request_id: request_id.into(),
            text: "Example".into(),
            html: Some("<b>Example</b>".into()),
            uri: None,
        };

        assert_eq!(
            serde_json::to_value(command).expect("command serializes"),
            serde_json::json!({
                "type": "clipboard_set", "protocol": 1, "request_id": request_id,
                "text": "Example", "html": "<b>Example</b>"
            })
        );
    }

    #[test]
    fn cancel_share_rejects_unknown_transfers() {
        let dir = tempfile::tempdir().unwrap();
        let backend = NativeBackend::open(dir.path().join("native")).unwrap();

        assert!(!backend.cancel_share("unknown-peer", "not-a-transfer-id"));
        assert!(!backend.cancel_share("unknown-peer", "0123456789abcdef0123456789abcdef"));
    }

    #[test]
    fn keep_awake_and_screensaver_control_carry_result_correlation() {
        let request_id = "0123456789abcdef0123456789abcdef";
        let keep_awake = Message::KeepAwake {
            protocol: WIRE_VERSION,
            request_id: request_id.into(),
            inhibit: true,
        };
        let control = Message::ScreensaverControl {
            protocol: WIRE_VERSION,
            request_id: request_id.into(),
            inhibit: false,
        };

        assert_eq!(
            serde_json::to_value(keep_awake).expect("command serializes"),
            serde_json::json!({
                "type": "keep_awake", "protocol": 1, "request_id": request_id,
                "inhibit": true
            })
        );
        assert_eq!(
            serde_json::to_value(control).expect("command serializes"),
            serde_json::json!({
                "type": "screensaver_control", "protocol": 1, "request_id": request_id,
                "inhibit": false
            })
        );
    }

    #[test]
    fn tethering_settings_opens_user_screen_without_claiming_sharing() {
        let request_id = "0123456789abcdef0123456789abcdef";
        let command = Message::TetheringSettings {
            protocol: WIRE_VERSION,
            request_id: request_id.into(),
        };

        assert_eq!(
            serde_json::to_value(command).expect("command serializes"),
            serde_json::json!({
                "type": "tethering_settings", "protocol": 1, "request_id": request_id
            })
        );
    }

    #[test]
    fn media_command_carries_result_correlation_id() {
        let request_id = "0123456789abcdef0123456789abcdef";
        let command = Message::MediaControl {
            protocol: WIRE_VERSION,
            request_id: request_id.into(),
            player: "player".into(),
            action: WireCommand::Play,
            position_ms: None,
        };

        assert_eq!(
            serde_json::to_value(command).expect("command serializes"),
            serde_json::json!({
                "type": "media_control", "protocol": 1, "request_id": request_id,
                "player": "player", "action": "play"
            })
        );
    }

    #[test]
    fn pending_results_expire_once_at_deadline() {
        let now = Instant::now();
        let mut pending = BTreeMap::from([
            ("old".to_owned(), now - SHARE_RESULT_TIMEOUT),
            ("fresh".to_owned(), now),
        ]);
        assert_eq!(take_expired_shares(&mut pending, now), vec!["old"]);
        assert_eq!(take_expired_shares(&mut pending, now), Vec::<String>::new());
        assert!(pending.contains_key("fresh"));
    }

    #[test]
    fn streaming_stops_at_transfer_deadline() {
        let mut output = Vec::new();
        let mut progress = Vec::new();
        let error = stream_file_until(
            &mut [1u8; 4].as_slice(),
            &mut output,
            4,
            Instant::now() - SHARE_RESULT_TIMEOUT,
            &mut |sent| progress.push(sent),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(output.is_empty());
        assert_eq!(progress, [0]);
        progress.clear();
        stream_file_until(
            &mut [1u8; 4].as_slice(),
            &mut output,
            4,
            Instant::now(),
            &mut |sent| progress.push(sent),
        )
        .unwrap();
        assert_eq!(output, [1, 1, 1, 1]);
        assert_eq!(progress, [0, 4]);
    }

    #[test]
    fn inbound_read_checks_deadline_between_partial_reads() {
        struct SlowReader {
            reads: usize,
        }

        impl Read for SlowReader {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                if self.reads > 0 {
                    std::thread::sleep(Duration::from_millis(20));
                }
                self.reads += 1;
                output[0] = 1;
                Ok(1)
            }
        }

        let mut reader = SlowReader { reads: 0 };
        let mut output = [0u8; 3];
        let error = read_exact_until(
            &mut reader,
            &mut output,
            Instant::now() + Duration::from_millis(10),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert_eq!(&output[..2], &[1, 1]);
    }

    #[test]
    fn real_socket_transfer_deadline_cleans_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let backend = NativeBackend::open(dir.path().join("native")).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback test listener");
        let address = listener.local_addr().unwrap();
        let writer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.write_all(&[1]).unwrap();
            socket.flush().unwrap();
            std::thread::sleep(Duration::from_millis(30));
        });
        let mut socket = TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let error = backend
            .receive_share_file_until(
                &mut socket,
                "slow.bin",
                2,
                Instant::now() + Duration::from_millis(10),
            )
            .unwrap_err();
        assert!(
            matches!(error, NativeError::Io(ref error) if error.kind() == std::io::ErrorKind::TimedOut)
        );
        writer.join().unwrap();
        let received = backend.directory.join("received");
        assert!(fs::read_dir(received).unwrap().next().is_none());
    }

    #[test]
    fn peer_certificate_validator_accepts_generated_identity() {
        let (_key, certificate) = generate_identity().unwrap();
        validate_peer_certificate(&certificate).unwrap();
    }

    #[test]
    fn source_admission_bounds_active_and_recent_connections() {
        let dir = tempfile::tempdir().unwrap();
        let backend = NativeBackend::open(dir.path().join("native")).unwrap();
        let source = "192.0.2.1".parse().unwrap();
        let now = Instant::now();
        let mut inner = backend.inner.lock().unwrap();
        for _ in 0..MAX_SESSIONS_PER_SOURCE {
            assert!(admit_source(&mut inner, source, now));
        }
        assert!(!admit_source(&mut inner, source, now));
        release_source(&mut inner, source);
        assert!(admit_source(&mut inner, source, now));
        for _ in 0..(MAX_ATTEMPTS_PER_SOURCE - MAX_SESSIONS_PER_SOURCE - 1) {
            release_source(&mut inner, source);
            assert!(admit_source(&mut inner, source, now));
        }
        release_source(&mut inner, source);
        assert!(!admit_source(&mut inner, source, now));
        assert!(admit_source(&mut inner, source, now + ATTEMPT_WINDOW));
    }

    #[test]
    fn malformed_frame_corpus_never_panics() {
        for length in 0..256usize {
            let mut bytes = vec![0u8; length];
            let mut state = length as u32 ^ 0x9e37_79b9;
            for byte in &mut bytes {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *byte = (state >> 24) as u8;
            }
            let _ = read_frame(&mut bytes.as_slice());
        }
    }

    #[test]
    fn received_quota_refuses_oversize_and_sweeps_expired() {
        let dir = tempfile::tempdir().unwrap();
        let backend = NativeBackend::open(dir.path().join("native")).unwrap();
        let received = dir.path().join("native").join("received");
        // A file larger than the whole quota is refused outright.
        assert!(matches!(
            backend.receive_share_file(
                &mut [0u8; 8].as_slice(),
                "huge.bin",
                MAX_RECEIVED_BYTES + 1
            ),
            Err(NativeError::InvalidFrame)
        ));
        // Fill the quota with aged files, then verify the next
        // receive evicts oldest-first instead of failing.
        std::fs::create_dir_all(received.join("old")).unwrap();
        let aged = std::time::SystemTime::now() - RECEIVED_TTL - std::time::Duration::from_secs(1);
        for name in ["a.bin", "b.bin"] {
            let path = received.join("old").join(name);
            std::fs::write(&path, vec![7u8; 1024]).unwrap();
            let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.set_modified(aged).unwrap();
        }
        let fresh = vec![9u8; 2048];
        let path = backend
            .receive_share_file(&mut fresh.as_slice(), "new.bin", fresh.len() as u64)
            .expect("quota makes room oldest-first");
        assert_eq!(fs::read(path).unwrap(), fresh);
        // TTL sweep took the aged files even though the quota had room.
        assert!(!received.join("old").join("a.bin").exists());
        assert!(!received.join("old").join("b.bin").exists());
    }

    #[test]
    fn cancelled_clipboard_share_deletes_its_temp() {
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join("clip.png");
        std::fs::write(&temp, b"pixels").unwrap();
        let user = dir.path().join("keep.txt");
        std::fs::write(&user, b"mine").unwrap();
        delete_clipboard_temp(&Message::ShareFile {
            protocol: WIRE_VERSION,
            transfer_id: "t".into(),
            name: "clip.png".into(),
            size: 6,
            clipboard: true,
            mime: Some("image/png".into()),
            path: temp.clone(),
        });
        assert!(!temp.exists());
        delete_clipboard_temp(&Message::ShareFile {
            protocol: WIRE_VERSION,
            transfer_id: "u".into(),
            name: "keep.txt".into(),
            size: 4,
            clipboard: false,
            mime: None,
            path: user.clone(),
        });
        assert!(user.exists());
    }

    #[test]
    fn received_file_streams_and_cleans_interruption() {
        let dir = tempfile::tempdir().unwrap();
        let backend = NativeBackend::open(dir.path().join("native")).unwrap();
        let bytes = vec![42u8; SHARE_BUFFER * 2 + 7];
        let path = backend
            .receive_share_file(&mut bytes.as_slice(), "space ✓.txt", bytes.len() as u64)
            .unwrap();
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert!(
            backend
                .receive_share_file(&mut [1u8; 3].as_slice(), "cut.txt", 4)
                .is_err()
        );
        let received = backend.directory.join("received");
        let entries = fs::read_dir(received)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert!(
            !fs::read_dir(entries[0].path())
                .unwrap()
                .any(|entry| { entry.unwrap().file_name().to_string_lossy() == ".partial" })
        );
        for name in ["../bad", "a/b", "a\\b", ".", "..", "bad\nname"] {
            assert!(!safe_share_name(name));
        }
        assert!(
            backend
                .receive_share_file(&mut [].as_slice(), "big", MAX_SHARE_SIZE + 1)
                .is_err()
        );
    }
    #[test]
    fn discovery_record_matches_advertised_service() {
        // Deterministic coverage for the DNS-SD advertisement shape.
        let id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let service = discovery_service(24837, id).unwrap();
        assert_eq!(service.get_type(), "_handover._tcp.local.");
        assert_eq!(
            service.get_fullname(),
            "Handover-0123456789ab._handover._tcp.local."
        );
        assert_eq!(service.get_port(), 24837);
        assert_eq!(
            service.get_properties().get_property_val_str("v"),
            Some("1")
        );
    }

    #[test]
    #[ignore = "requires a multicast-capable network interface"]
    fn live_mdns_advertise_and_browse() {
        let id = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let advertiser = advertise(41321, id).expect("start mDNS advertiser");
        let browser = ServiceDaemon::new().expect("start mDNS browser");
        let receiver = browser
            .browse("_handover._tcp.local.")
            .expect("browse for Handover services");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut resolved = false;
        while std::time::Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let Ok(event) = receiver.recv_timeout(remaining) else {
                break;
            };
            if let mdns_sd::ServiceEvent::ServiceResolved(info) = event {
                if info.get_port() == 41321 && info.get_fullname().starts_with("Handover-") {
                    resolved = true;
                    break;
                }
            }
        }
        browser.stop_browse("_handover._tcp.local.").ok();
        browser.shutdown().ok();
        advertiser.shutdown().ok();
        assert!(resolved, "advertised service was not resolved through mDNS");
    }
    #[test]
    fn framing_rejects_oversize() {
        let bytes = (MAX_FRAME as u32 + 1).to_be_bytes();
        assert!(matches!(
            read_frame(&mut &bytes[..]),
            Err(NativeError::InvalidFrame)
        ));
    }
    #[test]
    fn framing_rejects_truncated_and_wrong_version_messages() {
        let mut truncated = (6_u32).to_be_bytes().to_vec();
        truncated.extend_from_slice(br#"{"x":"#);
        assert!(matches!(
            read_frame(&mut &truncated[..]),
            Err(NativeError::InvalidFrame)
        ));

        let message = br#"{"type":"ping","protocol":2}"#;
        let mut wrong_version = (message.len() as u32).to_be_bytes().to_vec();
        wrong_version.extend_from_slice(message);
        assert!(matches!(
            read_frame(&mut &wrong_version[..]),
            Err(NativeError::InvalidFrame)
        ));
    }

    #[test]
    fn framing_round_trips_every_fixture_message() {
        let messages: Vec<Message> =
            serde_json::from_str(include_str!("../../../tests/fixtures/native-protocol.json"))
                .unwrap();
        for message in messages {
            let mut bytes = Vec::new();
            write_frame(&mut bytes, &message).unwrap();
            assert_eq!(read_frame(&mut &bytes[..]).unwrap(), message);
        }
    }
    #[test]
    fn code_matches_cross_language_vector_and_is_order_independent() {
        // Shared with NativeTransportTest on the Kotlin side: the same
        // fingerprints and nonces must produce this code in both languages.
        let fp_a = "aa".repeat(32);
        let fp_b = "bb".repeat(32);
        let nonce_a = "00112233445566778899aabbccddeeff";
        let nonce_b = "ffeeddccbbaa99887766554433221100";
        assert_eq!(comparison_code(&fp_a, nonce_a, &fp_b, nonce_b), "18954386");
        assert_eq!(
            comparison_code(&fp_a, nonce_a, &fp_b, nonce_b),
            comparison_code(&fp_b, nonce_b, &fp_a, nonce_a)
        );
    }
    #[test]
    fn identity_persists() {
        let dir = tempfile::tempdir().unwrap();
        let first = NativeBackend::open(dir.path().to_path_buf()).unwrap();
        let second = NativeBackend::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(first.identity(), second.identity());
    }

    #[test]
    fn unpair_revokes_persisted_trust_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let peer = Peer {
            id: "android-device-01".into(),
            name: "Pixel Test".into(),
            fingerprint: "fingerprint".into(),
        };
        fs::write(
            dir.path().join("peers.json"),
            serde_json::to_vec(&serde_json::json!({"peers": {peer.id.clone(): peer}})).unwrap(),
        )
        .unwrap();
        let backend = NativeBackend::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(backend.peers().len(), 1);
        assert!(backend.unpair("android-device-01").unwrap());
        assert!(!backend.unpair("android-device-01").unwrap());
        assert!(
            NativeBackend::open(dir.path().to_path_buf())
                .unwrap()
                .peers()
                .is_empty()
        );
    }

    fn test_peer() -> Peer {
        Peer {
            id: "android-device-01".into(),
            name: "Pixel Test".into(),
            fingerprint: "fingerprint".into(),
        }
    }

    fn wire_notification(key: &str) -> WireNotification {
        WireNotification {
            key: key.into(),
            app: "Example".into(),
            title: "Hello".into(),
            body: "World".into(),
            clearable: true,
            actions: vec![WireNotificationAction {
                id: "0".into(),
                label: "Reply".into(),
            }],
            reply_supported: true,
        }
    }

    #[test]
    fn native_notification_keeps_device_scoped_identity_without_content_in_debug() {
        let notification =
            normalize_native_notification(&test_peer(), &wire_notification("key-1")).unwrap();
        assert_eq!(
            notification.id,
            NotificationId::new(DeviceId::new("native:android-device-01"), "key-1")
        );
        assert_eq!(notification.actions.len(), 1);
        assert!(notification.reply_supported);
        assert!(notification.icon_path.is_none());
        // Reply tokens and action internals stay out of the normalized model;
        // only the advertised id/label pairs cross the boundary.
        let debug = format!("{notification:?}");
        assert!(debug.contains("Example"));
    }

    #[test]
    fn native_notification_rejects_bad_keys_sizes_and_duplicate_actions() {
        let peer = test_peer();
        assert!(normalize_native_notification(&peer, &wire_notification("")).is_err());
        let mut oversize = wire_notification("key-1");
        oversize.title = "t".repeat(MAX_NOTIFICATION_TITLE + 1);
        assert!(normalize_native_notification(&peer, &oversize).is_err());
        let mut dup = wire_notification("key-1");
        dup.actions.push(WireNotificationAction {
            id: "0".into(),
            label: "Again".into(),
        });
        assert!(normalize_native_notification(&peer, &dup).is_err());
        let mut many = wire_notification("key-1");
        many.actions = (0..MAX_NOTIFICATION_ACTIONS + 1)
            .map(|i| WireNotificationAction {
                id: i.to_string(),
                label: "Action".into(),
            })
            .collect();
        assert!(normalize_native_notification(&peer, &many).is_err());
    }

    #[test]
    fn notification_commands_queue_only_for_live_peers() {
        use handover_core::NotificationCommand;
        let dir = tempfile::tempdir().unwrap();
        let backend = NativeBackend::open(dir.path().to_path_buf()).unwrap();
        let id = NotificationId::new(DeviceId::new("native:android-device-01"), "key-1");
        // No session is active, so even a well-formed command is offline
        // rather than silently dropped: IPC maps this to a controlled error.
        assert!(matches!(
            backend.execute_notification(
                "android-device-01",
                &NotificationCommand::Dismiss {
                    notification_id: id.clone()
                }
            ),
            Err(NativeCommandError::Offline)
        ));
        assert!(!backend.request_notifications_sync("android-device-01"));
    }

    #[test]
    fn notifications_capability_follows_listener_permission() {
        assert!(
            device(&test_peer(), true, None, false, false)
                .capabilities
                .contains(&Capability::Battery)
        );
        assert!(
            !device(&test_peer(), true, None, false, false)
                .capabilities
                .contains(&Capability::Notifications)
        );
        assert!(
            device(&test_peer(), true, None, true, false)
                .capabilities
                .contains(&Capability::Notifications)
        );
    }

    #[test]
    fn media_capability_is_independent_of_notifications() {
        assert!(
            !device(&test_peer(), true, None, false, false)
                .capabilities
                .contains(&Capability::Media)
        );
        let both = device(&test_peer(), true, None, true, true).capabilities;
        assert!(both.contains(&Capability::Notifications));
        assert!(both.contains(&Capability::Media));
        let media_only = device(&test_peer(), true, None, false, true).capabilities;
        assert!(!media_only.contains(&Capability::Notifications));
        assert!(media_only.contains(&Capability::Media));
    }

    fn wire_media_session(player: &str) -> WireMediaSession {
        WireMediaSession {
            player: player.into(),
            application: "Test Player".into(),
            title: Some("Test track".into()),
            artist: Some("Test artist".into()),
            album: None,
            playback: WirePlayback::Playing,
            position_ms: Some(1_000),
            duration_ms: Some(180_000),
            controls: vec![
                WireControl::Play,
                WireControl::Pause,
                WireControl::SetPosition,
            ],
        }
    }

    #[test]
    fn native_media_keeps_device_scoped_identity_without_volume() {
        let session =
            normalize_native_media(&test_peer(), &wire_media_session("com.example")).unwrap();
        assert_eq!(
            session.id,
            MediaSessionId::new(DeviceId::new("native:android-device-01"), "com.example")
        );
        assert_eq!(session.application, "Test Player");
        assert_eq!(session.title.as_deref(), Some("Test track"));
        assert_eq!(session.playback, PlaybackState::Playing);
        assert_eq!(session.position_ms, Some(1_000));
        assert_eq!(session.duration_ms, Some(180_000));
        // Volume is read-only in the Handover model and never transported.
        assert_eq!(session.volume_percent, None);
        assert!(session.controls.contains(&MediaControl::Play));
        assert!(session.controls.contains(&MediaControl::SetPosition));
        assert!(!session.controls.contains(&MediaControl::Seek));
        // Track content stays out of debug formatting checks for secrets;
        // titles are content, so only assert structural fields here.
        assert!(session.album.is_none());
    }

    #[test]
    fn native_media_rejects_bad_players_sizes_and_advertised_seek() {
        let peer = test_peer();
        assert!(normalize_native_media(&peer, &wire_media_session("")).is_err());
        let mut oversize = wire_media_session("com.example");
        oversize.title = Some("t".repeat(MAX_MEDIA_TEXT + 1));
        assert!(normalize_native_media(&peer, &oversize).is_err());
        let mut big_position = wire_media_session("com.example");
        big_position.position_ms = Some(MAX_MEDIA_POSITION_MS + 1);
        assert!(normalize_native_media(&peer, &big_position).is_err());
        // Relative seeks have no genuine Android API: a phone advertising
        // one must not enter Handover state.
        let mut seek = wire_media_session("com.example");
        seek.controls.push(WireControl::Seek);
        assert!(normalize_native_media(&peer, &seek).is_err());
    }

    #[test]
    fn media_commands_queue_only_for_live_peers() {
        let dir = tempfile::tempdir().unwrap();
        let backend = NativeBackend::open(dir.path().to_path_buf()).unwrap();
        let id = MediaSessionId::new(DeviceId::new("native:android-device-01"), "com.example");
        assert!(matches!(
            backend.execute_media("android-device-01", &MediaCommand::Play { id: id.clone() }),
            Err(NativeCommandError::Offline)
        ));
        assert!(!backend.request_media_sync("android-device-01"));
    }
}
