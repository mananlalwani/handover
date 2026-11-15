use crate::DeviceId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeKind {
    Media,
    Ringtone,
    Alarm,
    Call,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeviceVolumeStream {
    pub kind: VolumeKind,
    pub min_level: u32,
    pub max_level: u32,
    pub level: u32,
    pub muted: bool,
    pub adjustable: bool,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeviceVolumeState {
    pub device_id: DeviceId,
    pub streams: Vec<DeviceVolumeStream>,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceVolumeFailure {
    PermissionRequired,
    InvalidValue,
    Unavailable,
    Unconfirmed,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeviceVolumeResult {
    pub device_id: DeviceId,
    pub request_id: String,
    pub accepted: bool,
    pub failure: Option<DeviceVolumeFailure>,
}
