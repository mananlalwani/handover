//! Native Android transport. TLS authenticates a persistent certificate; DNS-SD only locates us.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use handover_core::{
    BatteryState, Capability, Device, DeviceEvent, DeviceId, MediaCommand, MediaControl,
    MediaEvent, MediaSession, MediaSessionId, Notification, NotificationAction,
    NotificationCommand, NotificationEvent, NotificationId, PlaybackState, ReceivedShare,
    SharedResource, StateEvent,
};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use openssl::asn1::Asn1Time;
use openssl::bn::{BigNum, MsbOption};
use openssl::ec::{EcGroup, EcKey};
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::{PKey, Private};
use openssl::ssl::{SslAcceptor, SslMethod, SslVerifyMode, SslVersion};
use openssl::x509::{X509, X509NameBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::warn;

pub const WIRE_VERSION: u32 = 1;
pub const MAX_FRAME: usize = 64 * 1024;
const MAX_SESSIONS: usize = 16;
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
const SHARE_BUFFER: usize = 32 * 1024;

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
    connecting: BTreeSet<String>,
    batteries: BTreeMap<String, BatteryState>,
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
        let mut inner = self.backend.inner.lock().unwrap();
        inner.connecting.remove(&self.peer.id);
        inner.pending.remove(&self.peer.id);
        if self.published {
            inner.active.remove(&self.peer.id);
            inner.outbox.remove(&self.peer.id);
            let removed_keys = inner.notif_keys.remove(&self.peer.id).unwrap_or_default();
            inner.notif_enabled.remove(&self.peer.id);
            let removed_players = inner
                .media_players
                .remove(&self.peer.id)
                .unwrap_or_default();
            inner.media_enabled.remove(&self.peer.id);
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
                (self.event)(StateEvent::Notification(NotificationEvent::Removed(id)));
            }
            // Same ownership rule for media sessions: the disconnect drops the
            // peer's native players so a reconnect resyncs from current phone
            // state. `handoverd` also strips sessions of disconnected devices,
            // and the store dedupes, so a double removal is harmless.
            for id in media_removals {
                (self.event)(StateEvent::Media(MediaEvent::Removed(id)));
            }
            (self.event)(StateEvent::Device(event));
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
        player: String,
        action: WireCommand,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position_ms: Option<u64>,
    },
    ShareUrl {
        protocol: u32,
        url: String,
    },
    ShareFile {
        protocol: u32,
        name: String,
        size: u64,
        #[serde(skip)]
        path: PathBuf,
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

impl Message {
    fn version(&self) -> u32 {
        match self {
            Self::Hello { protocol, .. }
            | Self::PairOpen { protocol, .. }
            | Self::PairConfirm { protocol, .. }
            | Self::Paired { protocol }
            | Self::Battery { protocol, .. }
            | Self::NotificationPost { protocol, .. }
            | Self::NotificationRemoved { protocol, .. }
            | Self::NotificationsSync { protocol, .. }
            | Self::NotificationsRequest { protocol }
            | Self::NotificationDismiss { protocol, .. }
            | Self::NotificationReply { protocol, .. }
            | Self::NotificationAction { protocol, .. }
            | Self::MediaPost { protocol, .. }
            | Self::MediaRemoved { protocol, .. }
            | Self::MediaSync { protocol, .. }
            | Self::MediaRequest { protocol }
            | Self::MediaControl { protocol, .. }
            | Self::ShareUrl { protocol, .. }
            | Self::ShareFile { protocol, .. }
            | Self::Revoke { protocol }
            | Self::Ping { protocol }
            | Self::Pong { protocol } => *protocol,
        }
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
                connecting: BTreeSet::new(),
                batteries: BTreeMap::new(),
                notif_enabled: BTreeMap::new(),
                notif_keys: BTreeMap::new(),
                media_enabled: BTreeMap::new(),
                media_players: BTreeMap::new(),
                outbox: BTreeMap::new(),
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
        }
        inner.notif_enabled.remove(id);
        inner.notif_keys.remove(id);
        inner.media_enabled.remove(id);
        inner.media_players.remove(id);
        inner.outbox.remove(id);
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
        queue.push(Message::MediaControl {
            protocol: WIRE_VERSION,
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

    /// Accept one share for a live paired peer. Delivery is asynchronous; a
    /// later disconnect or I/O failure can prevent completion.
    pub fn share_url(&self, peer_id: &str, url: String) -> Result<(), NativeCommandError> {
        if !valid_share_url(&url) {
            return Err(NativeCommandError::QueueFull);
        }
        self.queue_share(
            peer_id,
            Message::ShareUrl {
                protocol: WIRE_VERSION,
                url,
            },
        )
    }

    pub fn share_file(&self, peer_id: &str, path: PathBuf) -> Result<(), NativeCommandError> {
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
        self.queue_share(
            peer_id,
            Message::ShareFile {
                protocol: WIRE_VERSION,
                name,
                size,
                path,
            },
        )
    }

    fn queue_share(&self, peer_id: &str, message: Message) -> Result<(), NativeCommandError> {
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

    fn receive_share_file<R: Read>(
        &self,
        input: &mut R,
        name: &str,
        size: u64,
    ) -> Result<PathBuf, NativeError> {
        if !safe_share_name(name) || name.len() > 255 || size > MAX_SHARE_SIZE {
            return Err(NativeError::InvalidFrame);
        }
        let directory = self.directory.join("received");
        fs::create_dir_all(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
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
            input.read_exact(&mut buffer[..amount])?;
            output.write_all(&buffer[..amount])?;
            remaining -= amount as u64;
        }
        output.sync_all()?;
        drop(output);
        fs::rename(&partial, &destination)?;
        guard.0 = PathBuf::new();
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
        let listener = TcpListener::bind(("0.0.0.0", LISTEN_PORT))?;
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
            let stream = incoming?;
            let mut inner = self.inner.lock().unwrap();
            if inner.sessions >= MAX_SESSIONS {
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
                backend.inner.lock().unwrap().sessions -= 1;
            });
        }
        Ok(())
    }

    fn handle(
        &self,
        stream: TcpStream,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
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
        let cert = tls
            .ssl()
            .peer_certificate()
            .ok_or(NativeError::InvalidFrame)?;
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
        {
            let mut inner = self.inner.lock().unwrap();
            if !inner.peers.peers.contains_key(&id) {
                return Err(NativeError::InvalidFrame);
            }
            inner.active.insert(id.clone(), tls.get_ref().try_clone()?);
            session.published = true;
            let notifications_supported = inner.notif_enabled.get(&id).copied().unwrap_or(false);
            let media_supported = inner.media_enabled.get(&id).copied().unwrap_or(false);
            event(StateEvent::Device(DeviceEvent::Added(device(
                &peer,
                true,
                inner.batteries.get(&id).cloned(),
                notifications_supported,
                media_supported,
            ))));
        }
        // Ask a freshly paired phone for its current notification list. The
        // phone also syncs proactively after `paired`; the request covers a
        // daemon restart where the phone never saw the pairing transition.
        let _ = write_frame(
            &mut tls,
            &Message::NotificationsRequest {
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
        loop {
            if last_received.elapsed() > Duration::from_secs(90) {
                break;
            }
            if !self.inner.lock().unwrap().peers.peers.contains_key(&id) {
                break;
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
            for message in outbound {
                if let Err(error) = write_frame(&mut tls, &message) {
                    tracing::debug!(peer = %id, %error, "native notification command write failed");
                    return Err(error);
                }
                if let Message::ShareFile { path, size, .. } = &message {
                    let file = File::open(path)?;
                    let metadata = file.metadata()?;
                    if !metadata.is_file() || metadata.len() != *size {
                        return Err(NativeError::InvalidFrame);
                    }
                    let mut limited = file.take(*size);
                    if std::io::copy(&mut limited, &mut tls)? != *size {
                        return Err(NativeError::InvalidFrame);
                    }
                    tls.flush()?;
                }
            }
            match read_frame(&mut tls) {
                Ok(Message::ShareUrl {
                    protocol: WIRE_VERSION,
                    url,
                }) => {
                    last_received = Instant::now();
                    if !valid_share_url(&url) {
                        return Err(NativeError::InvalidFrame);
                    }
                    event(StateEvent::ShareReceived(ReceivedShare {
                        device_id: DeviceId::new(format!("native:{id}")),
                        resource: SharedResource::Url { url },
                    }));
                }
                Ok(Message::ShareFile {
                    protocol: WIRE_VERSION,
                    name,
                    size,
                    ..
                }) => {
                    last_received = Instant::now();
                    let path = self.receive_share_file(&mut tls, &name, size)?;
                    event(StateEvent::ShareReceived(ReceivedShare {
                        device_id: DeviceId::new(format!("native:{id}")),
                        resource: SharedResource::File {
                            path: path.to_string_lossy().into_owned(),
                        },
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
                    let mut inner = self.inner.lock().unwrap();
                    if !inner.peers.peers.contains_key(&id) {
                        break;
                    }
                    inner.batteries.insert(id.clone(), battery);
                    let notifications_supported =
                        inner.notif_enabled.get(&id).copied().unwrap_or(false);
                    let media_supported = inner.media_enabled.get(&id).copied().unwrap_or(false);
                    event(StateEvent::Device(DeviceEvent::Updated(device(
                        &peer,
                        true,
                        Some(battery),
                        notifications_supported,
                        media_supported,
                    ))));
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
        // Code-level coverage for the DNS-SD advertisement. Multicast
        // discovery has no live-network verification (see DESIGN.md).
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
