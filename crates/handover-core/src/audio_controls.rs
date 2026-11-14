//! Laptop audio choices shared by receiving devices. IDs are opaque to clients.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AudioRoutingSettings {
    pub source: AudioSourceSelection,
    pub output: AudioOutputMode,
    pub playback: AudioPlaybackTarget,
}
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioSourceSelection {
    #[default]
    All,
    Applications {
        ids: Vec<String>,
    },
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioOutputMode {
    #[default]
    Phone,
    Laptop,
    Both,
}
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioPlaybackTarget {
    #[default]
    Automatic,
    Player {
        id: String,
    },
}
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AudioRoutingChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<AudioSourceSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<AudioOutputMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback: Option<AudioPlaybackTarget>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AudioApplication {
    pub id: String,
    pub name: String,
    pub available: bool,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AudioDesktopPlayer {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AudioControlsSnapshot {
    pub settings: AudioRoutingSettings,
    pub applications: Vec<AudioApplication>,
    pub players: Vec<AudioDesktopPlayer>,
    pub revision: u64,
    pub observed_at_ms: i64,
    pub applications_available: bool,
    pub players_available: bool,
    pub settings_available: bool,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioControlFailure {
    Unavailable,
    UnknownApplication,
    UnknownPlayer,
    RoutingFailed,
    InvalidSelection,
}
