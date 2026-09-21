//! Native Android transport. TLS authenticates a persistent certificate; DNS-SD only locates us.

mod discovery;
mod errors;
mod identity;
mod limits;
mod pairing;
mod protocol;
mod services;
mod session;
mod transfer;

pub(crate) use transfer::*;

pub use errors::{NativeCommandError, NativeError};
pub use limits::{MAX_FRAME, WIRE_VERSION};

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use handover_core::{
    BatteryState, CallAction, CallEvent, Capability, ConnectivityState, Device, DeviceEvent,
    DeviceId, MediaCommand, MediaEvent, MediaSessionId, NotificationCommand, NotificationEvent,
    NotificationId, PresentationCommand, RemoteInputCommand, ShareFailure, ShareResult,
    ShareStatus, StateEvent, VolumeCommand,
};
use openssl::pkey::{PKey, Private};
use openssl::x509::X509;
use serde::{Deserialize, Serialize};

use crate::discovery::*;
use crate::identity::*;
use crate::limits::*;
use crate::protocol::*;

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
pub(crate) struct PeerFile {
    pub(crate) peers: BTreeMap<String, Peer>,
}

pub(crate) struct Candidate {
    pub(crate) peer: Peer,
    pub(crate) code: String,
    pub(crate) commit: String,
    pub(crate) approved: bool,
    pub(crate) created: Instant,
}
pub(crate) struct Runtime {
    pub(crate) peers: PeerFile,
    pub(crate) pending: BTreeMap<String, Candidate>,
    pub(crate) active: BTreeMap<String, TcpStream>,
    pub(crate) sessions: usize,
    pub(crate) source_admissions: BTreeMap<IpAddr, SourceAdmission>,
    pub(crate) connecting: BTreeSet<String>,
    pub(crate) batteries: BTreeMap<String, BatteryState>,
    pub(crate) connectivity: BTreeMap<String, ConnectivityState>,
    // Native notification state, owned per paired peer. Keys are the Android
    // notification keys advertised as `local_id` in the normalized model.
    pub(crate) notif_enabled: BTreeMap<String, bool>,
    pub(crate) notif_keys: BTreeMap<String, BTreeSet<String>>,
    // Native media state, owned per paired peer. Players are the Android
    // package names advertised as `player_id` in the normalized model.
    pub(crate) media_enabled: BTreeMap<String, bool>,
    pub(crate) media_players: BTreeMap<String, BTreeSet<String>>,
    // Queued Linux-to-phone notification commands, drained by the owning
    // session thread. Bounded per peer; IPC reports acceptance, not delivery.
    pub(crate) outbox: BTreeMap<String, Vec<Message>>,
    pub(crate) pending_shares: BTreeMap<String, BTreeMap<String, Instant>>,
    // Latest call-state generation reported by each peer. A queued call
    // command stamped with an older generation is stale: a new call may have
    // started or ended since the user acted, so the command is dropped.
    pub(crate) call_generations: BTreeMap<String, u64>,
    pub(crate) pending_call_results: BTreeMap<String, BTreeMap<String, CallAction>>,
    // Peers that asked the desktop to stay awake. Cleared on disconnect so a
    // dead phone cannot hold the inhibitor past its session.
    pub(crate) screensaver_requests: BTreeSet<String>,
    /// In-flight received-share reservations (bytes, files). Disk scans
    /// cannot see concurrent transfers, so each transfer reserves
    /// before streaming and releases on failure; completed files stay
    /// counted by later scans.
    pub(crate) quota_reserved: (u64, usize),
}

#[derive(Default)]
pub(crate) struct SourceAdmission {
    pub(crate) active: usize,
    pub(crate) attempts: VecDeque<Instant>,
}

pub(crate) fn admit_source(inner: &mut Runtime, source: IpAddr, now: Instant) -> bool {
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

pub(crate) fn release_source(inner: &mut Runtime, source: IpAddr) {
    if let Some(admission) = inner.source_admissions.get_mut(&source) {
        admission.active = admission.active.saturating_sub(1);
    }
}

#[derive(Clone)]
pub struct NativeBackend {
    pub(crate) inner: Arc<Mutex<Runtime>>,
    pub(crate) directory: PathBuf,
    pub(crate) certificate: X509,
    pub(crate) key: PKey<Private>,
    pub(crate) id: String,
}

// Serializes session teardown with peer admission so an old disconnect cannot
// overwrite a replacement connection's presence.
pub(crate) struct Session<'a> {
    pub(crate) backend: &'a NativeBackend,
    pub(crate) peer: Peer,
    pub(crate) event: &'a Arc<dyn Fn(StateEvent) + Send + Sync>,
    pub(crate) published: bool,
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

/// Temp path owned by a queued clipboard share, if any. Only
/// `clipboard: true` entries name daemon-owned temp files; user files
/// must never be deleted by queue cleanup.
/// Reservation against the received-share quota. Dropping an
/// uncommitted reservation releases it; a completed transfer commits
/// it, leaving the on-disk file to future scans.
struct QuotaReservation {
    backend: NativeBackend,
    bytes: u64,
    committed: bool,
}

impl QuotaReservation {
    fn commit_rename(mut self, partial: &Path, destination: &Path) -> Result<(), NativeError> {
        let mut inner = self.backend.inner.lock().unwrap();
        // Keep the rename and reservation release under the same runtime
        // lock. Quota scans cannot observe the completed file while the
        // reservation still counts it.
        fs::rename(partial, destination)?;
        inner.quota_reserved.0 = inner.quota_reserved.0.saturating_sub(self.bytes);
        inner.quota_reserved.1 = inner.quota_reserved.1.saturating_sub(1);
        self.committed = true;
        Ok(())
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

struct PartialShare(PathBuf, PathBuf);

impl Drop for PartialShare {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.0);
            let _ = fs::remove_dir(&self.1);
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
        let html_len = html.as_ref().map_or(0, String::len);
        let uri_len = uri.as_ref().map_or(0, String::len);
        if text.len() > 32 * 1024
            || html_len > 32 * 1024
            || uri_len > 32 * 1024
            || text.len() + html_len + uri_len > 48 * 1024
        {
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

    /// Ask Android for a bounded directory listing. The response is delivered
    /// asynchronously on the native session and is not treated as daemon
    /// state.
    pub fn filesystem_list(
        &self,
        peer_id: &str,
        path: String,
    ) -> Result<String, NativeCommandError> {
        if !safe_browse_path(&path) {
            return Err(NativeCommandError::QueueFull);
        }
        let request_id = new_transfer_id().map_err(|_| NativeCommandError::QueueFull)?;
        self.queue_simple(
            peer_id,
            Message::FilesystemList {
                protocol: WIRE_VERSION,
                request_id: request_id.clone(),
                path,
            },
        )?;
        Ok(request_id)
    }

    pub(crate) fn queue_simple(
        &self,
        peer_id: &str,
        message: Message,
    ) -> Result<(), NativeCommandError> {
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

    pub(crate) fn queue_share(
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
    pub(crate) fn reserve_received_quota(
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
                    if child.file_name() == ".partial" {
                        // Active transfers are accounted for by the
                        // reservation table. Never evict their partial.
                        continue;
                    }
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

    pub(crate) fn receive_share_file<R: Read>(
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

    pub(crate) fn receive_share_file_until<R: Read>(
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
        reservation.commit_rename(&partial, &destination)?;
        guard.0 = PathBuf::new();
        Ok(destination)
    }
    pub(crate) fn save_peers(&self, peers: &PeerFile) -> Result<(), NativeError> {
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
}

pub(crate) fn device(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pairing::comparison_code;
    use crate::services::media::normalize_native_media;
    use crate::services::notifications::normalize_native_notification;
    use handover_core::{DeviceCommandAction, DeviceCommandFailure, MediaControl, PlaybackState};
    use mdns_sd::ServiceDaemon;

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

    #[test]
    fn browse_paths_reject_traversal_and_absolute_roots() {
        assert!(safe_browse_path("."));
        assert!(safe_browse_path("Documents"));
        assert!(safe_browse_path("Documents/notes"));
        assert!(!safe_browse_path(""));
        assert!(!safe_browse_path("/etc"));
        assert!(!safe_browse_path("../"));
        assert!(!safe_browse_path("foo/../bar"));
        assert!(!safe_browse_path("foo//bar"));
        assert!(!safe_browse_path("foo\\bar"));
    }

    #[test]
    fn directory_listings_stay_inside_the_configured_root() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("ok")).unwrap();
        fs::write(root.path().join("ok/note.txt"), b"hi").unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();

        let entries = list_directory_under(root.path().to_path_buf(), "ok").unwrap();
        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "note.txt" && !entry.directory)
        );
        assert!(list_directory_under(root.path().to_path_buf(), "/etc").is_err());
        assert!(list_directory_under(root.path().to_path_buf(), "../").is_err());
        assert!(list_directory_under(root.path().to_path_buf(), "escape").is_err());
    }

    #[test]
    fn custom_command_allowlist_matches_daemon_parser_rules() {
        let commands = parse_custom_commands(
            "# stay-awake helpers\n[[command]]\nname = \"lock-screen\"\nargv = [\"loginctl\", \"lock-session\"]\n\
             [[command]]\nname = \"Bad Name!\"\nargv = [\"x\"]\n\
             [[command]]\nname = \"ok\"\nargv = [\"true\"]\n",
        );
        assert_eq!(
            commands,
            vec![
                (
                    "lock-screen".to_owned(),
                    vec!["loginctl".to_owned(), "lock-session".to_owned()]
                ),
                ("ok".to_owned(), vec!["true".to_owned()]),
            ]
        );
        assert!(safe_command_name("lock-screen"));
        assert!(!safe_command_name("Bad Name!"));
        assert!(!safe_command_name("has.dot"));
    }

    #[test]
    fn clipboard_set_rejects_oversized_rich_payloads_without_a_live_peer() {
        let dir = tempfile::tempdir().unwrap();
        let backend = NativeBackend::open(dir.path().to_path_buf()).unwrap();
        let too_big = "x".repeat(32 * 1024 + 1);
        assert!(matches!(
            backend.clipboard_set("missing", &too_big, None, None),
            Err(NativeCommandError::QueueFull)
        ));
        assert!(matches!(
            backend.clipboard_set("missing", "ok", Some(too_big.clone()), None),
            Err(NativeCommandError::QueueFull)
        ));
        let combined = "y".repeat(24 * 1024);
        assert!(matches!(
            backend.clipboard_set(
                "missing",
                &combined,
                Some(combined.clone()),
                Some(combined.clone()),
            ),
            Err(NativeCommandError::QueueFull)
        ));
    }
}
