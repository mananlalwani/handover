use std::collections::BTreeSet;
use std::sync::Arc;

use handover_core::*;

use crate::limits::*;
use crate::protocol::*;
use crate::{NativeBackend, NativeError, Peer, device};

pub(crate) fn normalize_native_media(
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
    pub(crate) fn handle_media_post(
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

    pub(crate) fn handle_media_removed(
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

    pub(crate) fn handle_media_sync(
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
    pub(crate) fn clear_native_media(
        &self,
        peer: &Peer,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) {
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
