use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures_util::{StreamExt, stream};
use handover_core::{Capability, Device, DeviceId, MediaSession, MediaSessionId, PlaybackState};
use handover_ipc::{Client, ServerPayload};
use tokio::sync::{Semaphore, mpsc, watch};
use tokio::time::timeout;
use tracing::warn;
use zbus::{Connection, zvariant::Value};

use crate::media_player::{PATH, PLAYER, Player, ROOT, Root, State};
use crate::{CALL_TIMEOUT, Error, Reader};

const MAX_PLAYERS: usize = 32;
const MAX_DEVICES: usize = 64;

/// Export phone media on the supplied session bus, independently of notifications.
pub async fn run(path: PathBuf, address: String, mut shutdown: watch::Receiver<bool>) {
    let mut serial = 0u64;
    while !*shutdown.borrow() {
        if session(path.clone(), &address, &mut shutdown, &mut serial)
            .await
            .is_err()
        {
            warn!("desktop media session ended; reconnecting");
        }
        if *shutdown.borrow() {
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            changed = shutdown.changed() => { if changed.is_err() || *shutdown.borrow() { break; } }
        }
    }
}

struct Export {
    connection: Connection,
    state: Arc<RwLock<State>>,
}

impl Export {
    async fn update(&self, session: MediaSession, device_name: String) -> Result<(), Error> {
        let (changed, invalidated, identity) = {
            let mut state = self.state.write().unwrap();
            let old = &state.session;
            let metadata_changed = old.title != session.title
                || old.artist != session.artist
                || old.album != session.album
                || old.duration_ms != session.duration_ms;
            let status_changed = old.playback != session.playback;
            let volume_changed = old.volume_percent != session.volume_percent;
            let old_controls = old.controls.clone();
            if metadata_changed {
                state.revision = state
                    .revision
                    .checked_add(1)
                    .ok_or("metadata revision exhausted")?;
            }
            let identity_changed = state.device_name != device_name
                || state.session.application != session.application;
            state.session = session;
            state.device_name = device_name;
            let mut changed = HashMap::<&str, Value<'static>>::new();
            let mut invalidated = Vec::new();
            if status_changed {
                changed.insert("PlaybackStatus", state.status()?.into());
            }
            if metadata_changed {
                changed.insert("Metadata", state.metadata().into());
            }
            if volume_changed {
                if let Some(volume) = state.session.volume_percent.filter(|value| *value <= 100) {
                    changed.insert("Volume", (f64::from(volume) / 100.0).into());
                } else {
                    invalidated.push("Volume");
                }
            }
            for (name, control) in [
                ("CanPlay", handover_core::MediaControl::Play),
                ("CanPause", handover_core::MediaControl::Pause),
                ("CanGoNext", handover_core::MediaControl::Next),
                ("CanGoPrevious", handover_core::MediaControl::Previous),
            ] {
                if old_controls.contains(&control) != state.supports(control) {
                    changed.insert(name, state.supports(control).into());
                }
            }
            (
                changed,
                invalidated,
                identity_changed.then(|| state.identity()),
            )
        };
        if !changed.is_empty() || !invalidated.is_empty() {
            timeout(
                CALL_TIMEOUT,
                self.connection.emit_signal(
                    None::<&str>,
                    PATH,
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    &(PLAYER, changed, invalidated),
                ),
            )
            .await??;
        }
        if let Some(identity) = identity {
            let changed = HashMap::from([("Identity", Value::from(identity))]);
            timeout(
                CALL_TIMEOUT,
                self.connection.emit_signal(
                    None::<&str>,
                    PATH,
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    &(ROOT, changed, Vec::<String>::new()),
                ),
            )
            .await??;
        }
        Ok(())
    }
}

impl Drop for Export {
    fn drop(&mut self) {
        // In-flight object-server handlers may retain a connection clone. Deny
        // commands immediately, before async connection closure finishes.
        self.state.write().unwrap().active = false;
    }
}

struct Registry {
    path: PathBuf,
    address: String,
    devices: BTreeMap<DeviceId, Device>,
    sessions: BTreeMap<MediaSessionId, MediaSession>,
    exports: BTreeMap<MediaSessionId, Export>,
    commands: Arc<Semaphore>,
}

impl Registry {
    async fn refresh(&mut self, serial: &mut u64) -> Result<(), Error> {
        let wanted: BTreeSet<_> = self
            .sessions
            .values()
            .filter(|session| {
                session.playback != PlaybackState::Unknown
                    && self
                        .devices
                        .get(&session.id.device_id)
                        .is_some_and(|device| {
                            device.connected
                                && device.paired
                                && device.capabilities.contains(&Capability::Media)
                        })
            })
            .map(|session| session.id.clone())
            .collect();
        let removed: Vec<_> = self
            .exports
            .keys()
            .filter(|id| !wanted.contains(*id))
            .cloned()
            .collect();
        for id in removed {
            if let Some(export) = self.exports.remove(&id) {
                export.state.write().unwrap().active = false;
                let _ = timeout(CALL_TIMEOUT, export.connection.clone().close()).await;
            }
        }
        for id in wanted {
            let session = self.sessions[&id].clone();
            let name = self.devices[&id.device_id].name.clone();
            if let Some(export) = self.exports.get(&id) {
                let unchanged = {
                    let state = export.state.read().unwrap();
                    state.session == session && state.device_name == name
                };
                if !unchanged {
                    export.update(session, name).await?;
                }
            } else {
                *serial = serial.checked_add(1).ok_or("media player ID exhausted")?;
                let state = Arc::new(RwLock::new(State {
                    session,
                    device_name: name,
                    active: true,
                    revision: 1,
                }));
                let player = Player {
                    state: Arc::clone(&state),
                    socket: self.path.clone(),
                    commands: Arc::clone(&self.commands),
                };
                let connection = timeout(
                    CALL_TIMEOUT,
                    zbus::connection::Builder::address(self.address.as_str())?
                        .name(format!("org.mpris.MediaPlayer2.handover.player{serial}"))?
                        .serve_at(PATH, Root(Arc::clone(&state)))?
                        .serve_at(PATH, player)?
                        .build(),
                )
                .await??;
                self.exports.insert(id, Export { connection, state });
            }
        }
        Ok(())
    }

    async fn close(&mut self) {
        for export in self.exports.values() {
            export.state.write().unwrap().active = false;
        }
        let exports = std::mem::take(&mut self.exports);
        let _ = timeout(
            Duration::from_secs(2),
            stream::iter(exports.into_values()).for_each_concurrent(8, |export| async move {
                let _ = export.connection.clone().close().await;
            }),
        )
        .await;
    }
}

async fn session(
    path: PathBuf,
    address: &str,
    shutdown: &mut watch::Receiver<bool>,
    serial: &mut u64,
) -> Result<(), Error> {
    let mut subscription = timeout(CALL_TIMEOUT, async {
        Client::connect_to(path.clone())
            .await?
            .subscribe_media()
            .await
    })
    .await??;
    let (sender, mut receiver) = mpsc::channel(8);
    let initial = ServerPayload::Snapshot {
        devices: std::mem::take(&mut subscription.devices),
        notifications: vec![],
        media_sessions: std::mem::take(&mut subscription.media_sessions),
        calls: vec![],
        messaging_accounts: vec![],
        conversations: vec![],
        typing_states: vec![],
        read_states: vec![],
    };
    let _reader = Reader(tokio::spawn(async move {
        let mut subscription = subscription;
        if sender.send(initial).await.is_err() {
            return;
        }
        while let Ok(message) = subscription.next_message().await {
            if matches!(
                message.payload,
                ServerPayload::Snapshot { .. }
                    | ServerPayload::MediaAdded { .. }
                    | ServerPayload::MediaUpdated { .. }
                    | ServerPayload::MediaRemoved { .. }
                    | ServerPayload::DeviceAdded { .. }
                    | ServerPayload::DeviceUpdated { .. }
                    | ServerPayload::DeviceRemoved { .. }
            ) && sender.send(message.payload).await.is_err()
            {
                return;
            }
        }
    }));
    let mut registry = Registry {
        path,
        address: address.into(),
        devices: BTreeMap::new(),
        sessions: BTreeMap::new(),
        exports: BTreeMap::new(),
        commands: Arc::new(Semaphore::new(1)),
    };
    let outcome = async {
        loop {
            tokio::select! {
                update = receiver.recv() => {
                    match update.ok_or("daemon disconnected")? {
                        ServerPayload::Snapshot { devices, media_sessions, .. } => {
                            registry.devices = devices.into_iter().take(MAX_DEVICES).map(|device| (device.id.clone(), device)).collect();
                            registry.sessions = media_sessions.into_iter().take(MAX_PLAYERS).map(|session| (session.id.clone(), session)).collect();
                        }
                        ServerPayload::DeviceAdded { device } | ServerPayload::DeviceUpdated { device } => {
                            if registry.devices.contains_key(&device.id) || registry.devices.len() < MAX_DEVICES { registry.devices.insert(device.id.clone(), device); }
                        }
                        ServerPayload::DeviceRemoved { device_id } => {
                            registry.devices.remove(&device_id);
                            registry.sessions.retain(|id, _| id.device_id != device_id);
                        }
                        ServerPayload::MediaAdded { media_session } | ServerPayload::MediaUpdated { media_session } => {
                            if registry.sessions.contains_key(&media_session.id) || registry.sessions.len() < MAX_PLAYERS { registry.sessions.insert(media_session.id.clone(), media_session); }
                        }
                        ServerPayload::MediaRemoved { media_session_id } => { registry.sessions.remove(&media_session_id); }
                        _ => {}
                    }
                    timeout(CALL_TIMEOUT, registry.refresh(serial)).await??;
                }
                changed = shutdown.changed() => { if changed.is_err() || *shutdown.borrow() { return Ok::<(), Error>(()); } }
            }
        }
    }.await;
    registry.close().await;
    outcome
}
