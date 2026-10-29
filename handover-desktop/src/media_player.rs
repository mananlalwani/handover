use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use handover_core::{MediaCommand, MediaControl, MediaSession, PlaybackState};
use handover_ipc::Client;
use tokio::sync::Semaphore;
use tokio::time::timeout;
use zbus::{
    fdo, interface,
    zvariant::{OwnedObjectPath, Value},
};

use crate::CALL_TIMEOUT;

pub(crate) const PATH: &str = "/org/mpris/MediaPlayer2";
pub(crate) const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
pub(crate) const ROOT: &str = "org.mpris.MediaPlayer2";

pub(crate) struct State {
    pub session: MediaSession,
    pub device_name: String,
    pub active: bool,
    // A local reference to the observed metadata revision, never a backend
    // track identity. It cannot authorize seeking.
    pub revision: u64,
}

impl State {
    pub fn identity(&self) -> String {
        format!("{} on {}", self.session.application, self.device_name)
    }

    pub fn track_path(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(format!("/org/handover/Media/observed{}", self.revision))
            .expect("numeric metadata reference is a valid object path")
    }

    pub fn metadata(&self) -> HashMap<String, Value<'static>> {
        let mut result = HashMap::new();
        result.insert("mpris:trackid".into(), self.track_path().into());
        for (key, value) in [
            ("xesam:title", &self.session.title),
            ("xesam:album", &self.session.album),
        ] {
            if let Some(value) = value {
                result.insert(key.into(), value.clone().into());
            }
        }
        if let Some(artist) = &self.session.artist {
            result.insert("xesam:artist".into(), vec![artist.clone()].into());
        }
        if let Some(length) = self.session.duration_ms.and_then(microseconds) {
            result.insert("mpris:length".into(), length.into());
        }
        result
    }

    pub fn status(&self) -> fdo::Result<&'static str> {
        match self.session.playback {
            PlaybackState::Playing => Ok("Playing"),
            PlaybackState::Paused => Ok("Paused"),
            PlaybackState::Stopped => Ok("Stopped"),
            PlaybackState::Unknown => Err(unsupported("Playback state is unknown")),
        }
    }

    pub fn supports(&self, control: MediaControl) -> bool {
        self.active && self.session.controls.contains(&control)
    }
}

pub(crate) fn microseconds(milliseconds: u64) -> Option<i64> {
    milliseconds
        .checked_mul(1000)
        .and_then(|value| i64::try_from(value).ok())
}

pub(crate) fn unsupported(message: &str) -> fdo::Error {
    fdo::Error::NotSupported(message.into())
}

pub(crate) struct Root(pub Arc<RwLock<State>>);

#[interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {}
    fn quit(&self) -> fdo::Result<()> {
        Err(unsupported("Remote applications cannot be quit"))
    }
    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> String {
        self.0.read().unwrap().identity()
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        "handover"
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec![]
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        vec![]
    }
}

pub(crate) struct Player {
    pub state: Arc<RwLock<State>>,
    pub socket: PathBuf,
    pub commands: Arc<Semaphore>,
}

impl Player {
    async fn send(&self, control: MediaControl) -> fdo::Result<()> {
        let _permit = self
            .commands
            .try_acquire()
            .map_err(|_| fdo::Error::LimitsExceeded("Another media command is pending".into()))?;
        let command = {
            let state = self.state.read().unwrap();
            if !state.active {
                return Err(fdo::Error::Disconnected(
                    "Media session is no longer available".into(),
                ));
            }
            if !state.supports(control)
                || (control == MediaControl::PlayPause && !state.supports(MediaControl::Pause))
            {
                return Err(unsupported("Media control is unavailable"));
            }
            let id = state.session.id.clone();
            match control {
                MediaControl::Play => MediaCommand::Play { id },
                MediaControl::Pause => MediaCommand::Pause { id },
                MediaControl::PlayPause => MediaCommand::PlayPause { id },
                MediaControl::Next => MediaCommand::Next { id },
                MediaControl::Previous => MediaCommand::Previous { id },
                _ => return Err(unsupported("Seeking is unavailable")),
            }
        };
        // An IPC reply confirms admission only. Never mutate playback here or
        // retry a command whose eventual effect may be unknown.
        match timeout(CALL_TIMEOUT, async {
            Client::connect_to(self.socket.clone())
                .await?
                .media_command(command)
                .await
        })
        .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(fdo::Error::Failed(
                "Daemon did not accept the media command".into(),
            )),
            Err(_) => Err(fdo::Error::Timeout(
                "Media command outcome is unknown; it will not be retried".into(),
            )),
        }
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    async fn play(&self) -> fdo::Result<()> {
        self.send(MediaControl::Play).await
    }
    async fn pause(&self) -> fdo::Result<()> {
        self.send(MediaControl::Pause).await
    }
    async fn play_pause(&self) -> fdo::Result<()> {
        self.send(MediaControl::PlayPause).await
    }
    async fn next(&self) -> fdo::Result<()> {
        self.send(MediaControl::Next).await
    }
    async fn previous(&self) -> fdo::Result<()> {
        self.send(MediaControl::Previous).await
    }
    fn stop(&self) -> fdo::Result<()> {
        Err(unsupported("Stop is unavailable"))
    }
    fn seek(&self, _offset: i64) -> fdo::Result<()> {
        Err(unsupported("Seeking is unavailable"))
    }
    fn set_position(&self, _track_id: OwnedObjectPath, _position: i64) -> fdo::Result<()> {
        Err(unsupported("Seeking is unavailable"))
    }
    #[zbus(signal)]
    async fn seeked(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        position: i64,
    ) -> zbus::Result<()>;
    fn open_uri(&self, _uri: &str) -> fdo::Result<()> {
        Err(unsupported("Opening media is unavailable"))
    }
    #[zbus(property)]
    fn playback_status(&self) -> fdo::Result<String> {
        Ok(self.state.read().unwrap().status()?.into())
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, Value<'static>> {
        self.state.read().unwrap().metadata()
    }
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> fdo::Result<i64> {
        self.state
            .read()
            .unwrap()
            .session
            .position_ms
            .and_then(microseconds)
            .ok_or_else(|| unsupported("Position is unavailable"))
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn set_rate(&self, value: f64) -> zbus::Result<()> {
        if value == 1.0 {
            Ok(())
        } else {
            Err(unsupported("Playback rate cannot be changed").into())
        }
    }
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn volume(&self) -> fdo::Result<f64> {
        self.state
            .read()
            .unwrap()
            .session
            .volume_percent
            .filter(|value| *value <= 100)
            .map(|value| f64::from(value) / 100.0)
            .ok_or_else(|| unsupported("Phone volume is unavailable"))
    }
    #[zbus(property)]
    fn set_volume(&self, _value: f64) -> zbus::Result<()> {
        Err(unsupported("Phone volume cannot be changed").into())
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.state.read().unwrap().supports(MediaControl::Next)
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.state.read().unwrap().supports(MediaControl::Previous)
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.state.read().unwrap().supports(MediaControl::Play)
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.state.read().unwrap().supports(MediaControl::Pause)
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        false
    }
    #[zbus(property(emits_changed_signal = "false"))]
    fn can_control(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests;
