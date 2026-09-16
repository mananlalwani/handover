//! Backend-independent domain types used across Handover.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

/// A stable identifier assigned to a device by a backend.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct DeviceId(String);

impl DeviceId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Normalized state for a device known to Handover.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub connected: bool,
    pub paired: bool,
    pub battery: Option<BatteryState>,
    pub capabilities: BTreeSet<Capability>,
}

/// The battery information most recently reported by a device.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct BatteryState {
    percentage: u8,
    pub charging: bool,
}

impl<'de> Deserialize<'de> for BatteryState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SerializedBattery {
            percentage: u8,
            charging: bool,
        }

        let battery = SerializedBattery::deserialize(deserializer)?;
        Self::new(battery.percentage, battery.charging).map_err(serde::de::Error::custom)
    }
}

impl BatteryState {
    pub fn new(percentage: u8, charging: bool) -> Result<Self, BatteryStateError> {
        if percentage > 100 {
            return Err(BatteryStateError::InvalidPercentage(percentage));
        }

        Ok(Self {
            percentage,
            charging,
        })
    }

    pub fn percentage(self) -> u8 {
        self.percentage
    }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum BatteryStateError {
    #[error("battery percentage must be between 0 and 100, got {0}")]
    InvalidPercentage(u8),
}

/// A normalized feature that a device can provide.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Battery,
    FileTransfer,
    Notifications,
}

/// A change to the set of devices or to a device's normalized state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DeviceEvent {
    Added(Device),
    Updated(Device),
    Removed(DeviceId),
}

/// An identifier scoped by the device that produced the notification.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct NotificationId {
    pub device_id: DeviceId,
    pub local_id: String,
}

impl NotificationId {
    pub fn new(device_id: DeviceId, local_id: impl Into<String>) -> Self {
        Self {
            device_id,
            local_id: local_id.into(),
        }
    }
}

impl fmt::Display for NotificationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.device_id, self.local_id)
    }
}

/// An opaque action identifier and its user-visible label.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
}

/// Current normalized state for a notification originating from a device.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Notification {
    pub id: NotificationId,
    pub app_name: String,
    pub title: String,
    pub body: String,
    pub icon_path: Option<String>,
    pub clearable: bool,
    pub actions: Vec<NotificationAction>,
    pub reply_supported: bool,
}

/// A change to the daemon's current set of remote notifications.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum NotificationEvent {
    Added(Notification),
    Updated(Notification),
    Removed(NotificationId),
}

/// A backend-independent operation requested for a current notification.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum NotificationCommand {
    Dismiss {
        notification_id: NotificationId,
    },
    InvokeAction {
        notification_id: NotificationId,
        action_id: String,
    },
    Reply {
        notification_id: NotificationId,
        text: String,
    },
}

impl NotificationCommand {
    pub fn notification_id(&self) -> &NotificationId {
        match self {
            Self::Dismiss { notification_id }
            | Self::InvokeAction {
                notification_id, ..
            }
            | Self::Reply {
                notification_id, ..
            } => notification_id,
        }
    }
}

/// A normalized device or notification state transition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum StateEvent {
    Device(DeviceEvent),
    Notification(NotificationEvent),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_preserves_backend_identifier() {
        let id = DeviceId::new("phone-123");

        assert_eq!(id.as_str(), "phone-123");
        assert_eq!(id.to_string(), "phone-123");
    }

    #[test]
    fn battery_state_accepts_boundary_percentages() {
        let empty = BatteryState::new(0, false).expect("zero is a valid percentage");
        let full = BatteryState::new(100, true).expect("100 is a valid percentage");

        assert_eq!(empty.percentage(), 0);
        assert!(!empty.charging);
        assert_eq!(full.percentage(), 100);
        assert!(full.charging);
    }

    #[test]
    fn battery_state_rejects_percentage_over_100() {
        assert_eq!(
            BatteryState::new(101, false),
            Err(BatteryStateError::InvalidPercentage(101))
        );
    }

    #[test]
    fn battery_state_deserialization_preserves_invariant() {
        let error = serde_json::from_str::<BatteryState>(r#"{"percentage":101,"charging":false}"#)
            .expect_err("invalid battery must not deserialize");

        assert!(error.to_string().contains("between 0 and 100"));
    }

    #[test]
    fn device_event_carries_normalized_device_state() {
        let device = Device {
            id: DeviceId::new("phone-123"),
            name: "Phone".into(),
            connected: true,
            paired: true,
            battery: Some(BatteryState::new(82, false).expect("valid battery state")),
            capabilities: BTreeSet::from([Capability::Battery]),
        };

        assert_eq!(
            DeviceEvent::Added(device.clone()),
            DeviceEvent::Added(device)
        );
    }

    #[test]
    fn notification_identity_is_scoped_to_its_device() {
        let first = NotificationId::new(DeviceId::new("phone-a"), "42");
        let second = NotificationId::new(DeviceId::new("phone-b"), "42");

        assert_ne!(first, second);
        assert_eq!(first.to_string(), "phone-a:42");
    }
}
