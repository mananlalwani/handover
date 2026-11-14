//! Backend-independent audio intent, confirmed receiver state, and observations.
use crate::DeviceId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AudioStreamStatus {
    pub device_id: DeviceId,
    pub state: AudioStreamPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<AudioProfile>,
    #[serde(default)]
    pub retry_attempt: u32,
    #[serde(default)]
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<AudioStreamFailure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<AudioStreamDiagnostics>,
}
impl AudioStreamStatus {
    pub fn idle(device_id: DeviceId) -> Self {
        Self {
            device_id,
            state: AudioStreamPhase::Idle,
            enabled: Some(false),
            profile: Some(AudioProfile::Balanced),
            retry_attempt: 0,
            revision: 0,
            failure: None,
            diagnostics: None,
        }
    }
}

/// Streaming confirms receiver output started, not that sound reached a listener.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioStreamPhase {
    Idle,
    Starting,
    Streaming,
    Reconnecting,
    Suspended,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioProfile {
    #[default]
    Balanced,
    Music,
    LowDelay,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioStreamFailure {
    Disconnected,
    ConnectionFailed,
    CaptureFailed,
    StartupTimeout,
    AudioFocus,
    ReceiverFailed,
    RetriesExhausted,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AudioStreamDiagnostics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<AudioSenderDiagnostics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver: Option<AudioReceiverDiagnostics>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AudioSenderDiagnostics {
    pub encoder_bitrate_bps: u32,
    pub packet_duration_ms: u16,
    pub capture_dropped_packets: u64,
    pub updated_at_ms: i64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AudioReceiverDiagnostics {
    pub route: AudioRoute,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<AudioTransport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round_trip_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter_buffer_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate_bps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packets_received: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packets_lost: Option<u64>,
    #[serde(default)]
    pub updated_at_ms: i64,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioRoute {
    Direct,
    Vpn,
    Relay,
    Unknown,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioTransport {
    Udp,
    Tcp,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_status_does_not_invent_intent_profile_or_measurements() {
        let status: AudioStreamStatus =
            serde_json::from_str(r#"{"device_id":"phone","state":"streaming"}"#).unwrap();
        assert_eq!(status.state, AudioStreamPhase::Streaming);
        assert_eq!(status.enabled, None);
        assert_eq!(status.profile, None);
        assert!(status.diagnostics.is_none());
    }
}
