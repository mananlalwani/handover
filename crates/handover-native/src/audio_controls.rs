//! Resolve desktop-specific identities below the public audio controls contract.
use crate::{NativeBackend, WIRE_VERSION, audio::Event, protocol::Message};
use handover_core::{
    AudioControlFailure, AudioControlsSnapshot, AudioDesktopPlayer, AudioPlaybackTarget,
    AudioRoutingChange, AudioSourceSelection, DeviceId, StateEvent,
};
use sha2::{Digest, Sha256};
use std::net::Shutdown;

fn players_from_output(
    bytes: &[u8],
) -> Result<Vec<(AudioDesktopPlayer, String)>, AudioControlFailure> {
    let text = std::str::from_utf8(bytes).map_err(|_| AudioControlFailure::Unavailable)?;
    let mut players = Vec::new();
    for name in text.lines().filter(|name| !name.is_empty()) {
        if name.len() > 256
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        {
            continue;
        }
        if name == "playerctld" || name.to_ascii_lowercase().starts_with("handover") {
            continue;
        }
        let id = format!("player-{}", hex::encode(Sha256::digest(name.as_bytes())));
        players.push((
            AudioDesktopPlayer {
                id,
                name: name.to_owned(),
            },
            name.to_owned(),
        ));
        if players.len() > 256 {
            return Err(AudioControlFailure::Unavailable);
        }
    }
    players.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    players.dedup_by(|a, b| a.0.id == b.0.id);
    Ok(players)
}
fn players() -> Result<Vec<(AudioDesktopPlayer, String)>, AudioControlFailure> {
    let bytes = crate::audio_output::run_command("playerctl", &["-l"])
        .map_err(|_| AudioControlFailure::Unavailable)?;
    players_from_output(&bytes)
}
impl NativeBackend {
    /// Blocking host discovery and routing. Call only from the bounded local-command worker.
    pub fn audio_controls(
        &self,
        change: Option<AudioRoutingChange>,
    ) -> Result<AudioControlsSnapshot, AudioControlFailure> {
        let players = players();
        let mut router = self.audio_output.lock().unwrap();
        let applications = router.applications();
        if let Some(change) = change {
            let mut settings = router.settings.clone();
            if let Some(source) = change.source {
                if let AudioSourceSelection::Applications { ids } = &source {
                    if ids.len() > 64 {
                        return Err(AudioControlFailure::InvalidSelection);
                    }
                    let apps = applications
                        .as_ref()
                        .map_err(|_| AudioControlFailure::Unavailable)?;
                    if ids.iter().any(|id| !apps.iter().any(|app| app.id == *id)) {
                        return Err(AudioControlFailure::UnknownApplication);
                    }
                    let mut unique = ids.clone();
                    unique.sort();
                    unique.dedup();
                    if unique.len() != ids.len() {
                        return Err(AudioControlFailure::InvalidSelection);
                    }
                }
                settings.source = source;
            }
            if let Some(output) = change.output {
                settings.output = output;
            }
            if let Some(playback) = change.playback {
                if let AudioPlaybackTarget::Player { id } = &playback
                    && !players
                        .as_ref()
                        .map_err(|_| AudioControlFailure::Unavailable)?
                        .iter()
                        .any(|(player, _)| player.id == *id)
                {
                    return Err(AudioControlFailure::UnknownPlayer);
                }
                settings.playback = playback;
            }
            router
                .configure(settings)
                .map_err(|_| AudioControlFailure::RoutingFailed)?;
        }
        router.revision = router.revision.saturating_add(1);
        let mut snapshot = AudioControlsSnapshot {
            settings: router.settings.clone(),
            applications_available: applications.is_ok(),
            players_available: players.is_ok(),
            applications: applications.unwrap_or_default(),
            players: players
                .unwrap_or_default()
                .into_iter()
                .map(|(player, _)| player)
                .collect(),
            revision: router.revision,
            observed_at_ms: jiff::Timestamp::now().as_millisecond(),
            settings_available: router.settings_available,
        };
        // A catalog must fit the authenticated transport's frame limit. Do not
        // disconnect a stream merely because desktop discovery found too much.
        if serde_json::to_vec(&snapshot).map_or(true, |bytes| bytes.len() > crate::MAX_FRAME - 1024)
        {
            snapshot.applications_available = false;
            snapshot.applications.clear();
        }
        if serde_json::to_vec(&snapshot).map_or(true, |bytes| bytes.len() > crate::MAX_FRAME - 1024)
        {
            snapshot.players_available = false;
            snapshot.players.clear();
        }
        router.cached = Some(snapshot.clone());
        Ok(snapshot)
    }
    pub fn audio_controls_from_phone(
        &self,
        device_id: &DeviceId,
        change: Option<AudioRoutingChange>,
    ) -> Result<AudioControlsSnapshot, AudioControlFailure> {
        let peer = device_id
            .as_str()
            .strip_prefix("native:")
            .ok_or(AudioControlFailure::Unavailable)?;
        {
            let inner = self.inner.lock().unwrap();
            if !inner.active.contains_key(peer) || !inner.peers.peers.contains_key(peer) {
                return Err(AudioControlFailure::Unavailable);
            }
        }
        self.audio_controls(change)
    }
    pub fn cached_audio_controls(&self) -> Option<AudioControlsSnapshot> {
        self.audio_output.lock().unwrap().cached.clone()
    }
    pub fn publish_audio_controls(&self, snapshot: AudioControlsSnapshot, event: &Event) {
        let mut inner = self.inner.lock().unwrap();
        let peers: Vec<_> = inner
            .active
            .keys()
            .filter(|peer| inner.audio.get(*peer).is_some_and(|audio| audio.managed))
            .cloned()
            .collect();
        for peer in peers {
            let queue = inner.outbox.entry(peer.clone()).or_default();
            queue.retain(|message| !matches!(message, Message::AudioControlsUpdated { .. }));
            if queue.len() < crate::limits::MAX_OUTBOX_PER_PEER {
                queue.push(Message::AudioControlsUpdated {
                    protocol: WIRE_VERSION,
                    snapshot: snapshot.clone(),
                });
            } else if let Some(socket) = inner.active.get(&peer) {
                let _ = socket.shutdown(Shutdown::Both);
            }
        }
        drop(inner);
        event(StateEvent::AudioControlsUpdated(snapshot));
    }
    pub fn reply_audio_controls(
        &self,
        peer: &DeviceId,
        request_id: String,
        result: Result<AudioControlsSnapshot, AudioControlFailure>,
        event: &Event,
    ) {
        if let Ok(snapshot) = &result {
            self.publish_audio_controls(snapshot.clone(), event);
        }
        let Some(peer) = peer.as_str().strip_prefix("native:") else {
            return;
        };
        let (snapshot, failure) = match result {
            Ok(snapshot) => (Some(snapshot), None),
            Err(failure) => (None, Some(failure)),
        };
        let mut inner = self.inner.lock().unwrap();
        if !inner.active.contains_key(peer) {
            return;
        }
        let queue = inner.outbox.entry(peer.to_owned()).or_default();
        if queue.len() < crate::limits::MAX_OUTBOX_PER_PEER {
            queue.push(Message::AudioControlsResult {
                protocol: WIRE_VERSION,
                request_id,
                snapshot,
                failure,
            });
        } else if let Some(socket) = inner.active.get(peer) {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
    pub fn execute_audio_playback(
        &self,
        device_id: &DeviceId,
        stream_id: &str,
        playback: &AudioPlaybackTarget,
    ) -> bool {
        let Some(peer) = device_id.as_str().strip_prefix("native:") else {
            return false;
        };
        if !self.audio_stream_is_playing(peer, stream_id) {
            return false;
        }
        let target = match playback {
            AudioPlaybackTarget::Automatic => "playerctld,%any".to_owned(),
            AudioPlaybackTarget::Player { id } => {
                let Ok(players) = players() else {
                    return false;
                };
                let Some((_, name)) = players.into_iter().find(|(player, _)| player.id == *id)
                else {
                    return false;
                };
                name
            }
        };
        if !self.audio_stream_is_playing(peer, stream_id) {
            return false;
        }
        crate::audio_output::run_command(
            "playerctl",
            &[
                "--ignore-player=handover",
                &format!("--player={target}"),
                "play-pause",
            ],
        )
        .is_ok()
    }
    pub fn reply_audio_playback(
        &self,
        peer: &DeviceId,
        request_id: Option<String>,
        accepted: bool,
    ) {
        if let (Some(peer), Some(request_id)) = (peer.as_str().strip_prefix("native:"), request_id)
        {
            self.audio_stream_result(peer, request_id, accepted);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_controls_cannot_run_for_a_disconnected_or_unpaired_phone() {
        let directory = tempfile::tempdir().unwrap();
        let backend = NativeBackend::open(directory.path().to_path_buf()).unwrap();
        assert_eq!(
            backend.audio_controls_from_phone(&DeviceId::new("native:absent"), None),
            Err(AudioControlFailure::Unavailable)
        );
        assert!(backend.cached_audio_controls().is_none());
    }
    #[test]
    fn players_are_exact_instances_without_selector_syntax() {
        let players = players_from_output(
            b"spotify\nfirefox.instance1\nplayerctld\nhandover\n%any\nx,y\nspotify\n",
        )
        .unwrap();
        assert_eq!(players.len(), 2);
        assert!(
            players
                .iter()
                .all(|(player, name)| player.id.starts_with("player-") && !name.contains(','))
        );
        assert_ne!(players[0].0.id, players[1].0.id);
    }
}
