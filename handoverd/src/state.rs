use std::collections::BTreeMap;

use handover_core::{
    BatteryState, Capability, Device, DeviceEvent, DeviceId, MediaCommand, MediaEvent,
    MediaSession, MediaSessionId, Notification, NotificationCommand, NotificationEvent,
    NotificationId, ReceivedShare, StateEvent,
};

#[derive(Default)]
pub(crate) struct StateStore {
    devices: BTreeMap<DeviceId, Device>,
    notifications: BTreeMap<NotificationId, Notification>,
    media_sessions: BTreeMap<MediaSessionId, MediaSession>,
}

impl StateStore {
    pub(crate) fn apply(&mut self, event: StateEvent) -> ApplyOutcome {
        match event {
            StateEvent::Device(event) => self.apply_device(event),
            StateEvent::Notification(event) => self.apply_notification(event),
            StateEvent::Media(event) => self.apply_media(event),
            StateEvent::ShareReceived(share) => ApplyOutcome {
                changed: true,
                changes: vec![StateChange::ShareReceived(share)],
            },
        }
    }

    pub(crate) fn snapshot(&self) -> StateSnapshot {
        StateSnapshot {
            devices: self.devices.values().cloned().collect(),
            notifications: self.notifications.values().cloned().collect(),
            media_sessions: self.media_sessions.values().cloned().collect(),
        }
    }

    pub(crate) fn validate_command(
        &self,
        command: &NotificationCommand,
    ) -> Result<(), CommandValidationError> {
        let notification = self
            .notifications
            .get(command.notification_id())
            .ok_or(CommandValidationError::NotFound)?;
        match command {
            NotificationCommand::Dismiss { .. } if !notification.clearable => {
                Err(CommandValidationError::NotClearable)
            }
            NotificationCommand::InvokeAction { action_id, .. }
                if !notification
                    .actions
                    .iter()
                    .any(|action| &action.id == action_id) =>
            {
                Err(CommandValidationError::UnknownAction)
            }
            NotificationCommand::Reply { .. } if !notification.reply_supported => {
                Err(CommandValidationError::ReplyUnsupported)
            }
            NotificationCommand::Reply { text, .. } if text.trim().is_empty() => {
                Err(CommandValidationError::EmptyReply)
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn validate_media_command(
        &self,
        command: &MediaCommand,
    ) -> Result<(), MediaValidationError> {
        let session = self
            .media_sessions
            .get(command.id())
            .ok_or(MediaValidationError::NotFound)?;
        let device = self
            .devices
            .get(&session.id.device_id)
            .ok_or(MediaValidationError::DeviceUnavailable)?;
        if !device.connected || !device.paired {
            return Err(MediaValidationError::DeviceUnavailable);
        }
        if !device.capabilities.contains(&Capability::Media)
            || !session.controls.contains(&command.control())
        {
            return Err(MediaValidationError::UnsupportedControl);
        }
        match command {
            MediaCommand::SetPosition { position_ms, .. }
                if i32::try_from(*position_ms).is_err() =>
            {
                Err(MediaValidationError::InvalidValue)
            }
            MediaCommand::Seek { offset_ms, .. } if i32::try_from(*offset_ms).is_err() => {
                Err(MediaValidationError::InvalidValue)
            }
            _ => Ok(()),
        }
    }

    fn apply_media(&mut self, event: MediaEvent) -> ApplyOutcome {
        match event {
            MediaEvent::Added(session) | MediaEvent::Updated(session) => {
                let change = match self.media_sessions.get(&session.id) {
                    None => MediaChange::Added(session.id.clone()),
                    Some(previous) if previous == &session => return ApplyOutcome::unchanged(),
                    Some(_) => MediaChange::Updated(session.id.clone()),
                };
                self.media_sessions.insert(session.id.clone(), session);
                ApplyOutcome {
                    changed: true,
                    changes: vec![StateChange::Media(change)],
                }
            }
            MediaEvent::Removed(id) => {
                if self.media_sessions.remove(&id).is_some() {
                    ApplyOutcome {
                        changed: true,
                        changes: vec![StateChange::Media(MediaChange::Removed(id))],
                    }
                } else {
                    ApplyOutcome::unchanged()
                }
            }
        }
    }

    fn apply_device(&mut self, event: DeviceEvent) -> ApplyOutcome {
        match event {
            DeviceEvent::Added(device) | DeviceEvent::Updated(device) => self.upsert_device(device),
            DeviceEvent::Removed(id) => self.remove_device(id),
        }
    }

    fn apply_notification(&mut self, event: NotificationEvent) -> ApplyOutcome {
        match event {
            NotificationEvent::Added(notification) | NotificationEvent::Updated(notification) => {
                self.upsert_notification(notification)
            }
            NotificationEvent::Removed(id) => self.remove_notification(id),
        }
    }

    fn upsert_device(&mut self, device: Device) -> ApplyOutcome {
        let changes = match self.devices.get(&device.id) {
            None => initial_changes(&device),
            Some(previous) if previous == &device => return ApplyOutcome::unchanged(),
            Some(previous) => updated_changes(previous, &device),
        };
        self.devices.insert(device.id.clone(), device);
        ApplyOutcome {
            changed: true,
            changes,
        }
    }

    fn remove_device(&mut self, id: DeviceId) -> ApplyOutcome {
        match self.devices.remove(&id) {
            Some(device) => ApplyOutcome {
                changed: true,
                changes: vec![StateChange::Device(DeviceChange::Removed {
                    id,
                    name: device.name,
                })],
            },
            None => ApplyOutcome::unchanged(),
        }
    }

    fn upsert_notification(&mut self, notification: Notification) -> ApplyOutcome {
        let change = match self.notifications.get(&notification.id) {
            None => NotificationChange::Added(notification.clone()),
            Some(previous) if previous == &notification => return ApplyOutcome::unchanged(),
            Some(_) => NotificationChange::Updated(notification.clone()),
        };
        self.notifications
            .insert(notification.id.clone(), notification);
        ApplyOutcome {
            changed: true,
            changes: vec![StateChange::Notification(change)],
        }
    }

    fn remove_notification(&mut self, id: NotificationId) -> ApplyOutcome {
        match self.notifications.remove(&id) {
            Some(notification) => ApplyOutcome {
                changed: true,
                changes: vec![StateChange::Notification(NotificationChange::Removed {
                    id,
                    app_name: notification.app_name,
                })],
            },
            None => ApplyOutcome::unchanged(),
        }
    }

    pub(crate) fn get_device(&self, id: &DeviceId) -> Option<&Device> {
        self.devices.get(id)
    }

    #[cfg(test)]
    fn get_notification(&self, id: &NotificationId) -> Option<&Notification> {
        self.notifications.get(id)
    }
}

pub(crate) struct StateSnapshot {
    pub(crate) devices: Vec<Device>,
    pub(crate) notifications: Vec<Notification>,
    pub(crate) media_sessions: Vec<MediaSession>,
}

pub(crate) struct ApplyOutcome {
    pub(crate) changed: bool,
    pub(crate) changes: Vec<StateChange>,
}

impl ApplyOutcome {
    fn unchanged() -> Self {
        Self {
            changed: false,
            changes: Vec::new(),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum DeviceChange {
    Discovered {
        id: DeviceId,
        name: String,
        paired: bool,
    },
    NameChanged {
        id: DeviceId,
        old: String,
        new: String,
    },
    Connected {
        id: DeviceId,
        name: String,
    },
    Disconnected {
        id: DeviceId,
        name: String,
    },
    PairingChanged {
        id: DeviceId,
        name: String,
        paired: bool,
    },
    BatteryChanged {
        id: DeviceId,
        name: String,
        battery: Option<BatteryState>,
    },
    Removed {
        id: DeviceId,
        name: String,
    },
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum NotificationChange {
    Added(Notification),
    Updated(Notification),
    Removed {
        id: NotificationId,
        app_name: String,
    },
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum StateChange {
    Device(DeviceChange),
    Notification(NotificationChange),
    Media(MediaChange),
    ShareReceived(ReceivedShare),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum MediaChange {
    Added(MediaSessionId),
    Updated(MediaSessionId),
    Removed(MediaSessionId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MediaValidationError {
    NotFound,
    DeviceUnavailable,
    UnsupportedControl,
    InvalidValue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandValidationError {
    NotFound,
    NotClearable,
    UnknownAction,
    ReplyUnsupported,
    EmptyReply,
}

fn initial_changes(device: &Device) -> Vec<StateChange> {
    let mut changes = vec![DeviceChange::Discovered {
        id: device.id.clone(),
        name: device.name.clone(),
        paired: device.paired,
    }];
    if device.connected {
        changes.push(DeviceChange::Connected {
            id: device.id.clone(),
            name: device.name.clone(),
        });
    }
    if device.battery.is_some() {
        changes.push(DeviceChange::BatteryChanged {
            id: device.id.clone(),
            name: device.name.clone(),
            battery: device.battery,
        });
    }
    changes.into_iter().map(StateChange::Device).collect()
}

fn updated_changes(previous: &Device, current: &Device) -> Vec<StateChange> {
    let mut changes = Vec::new();
    if previous.name != current.name {
        changes.push(DeviceChange::NameChanged {
            id: current.id.clone(),
            old: previous.name.clone(),
            new: current.name.clone(),
        });
    }
    if previous.connected != current.connected {
        changes.push(if current.connected {
            DeviceChange::Connected {
                id: current.id.clone(),
                name: current.name.clone(),
            }
        } else {
            DeviceChange::Disconnected {
                id: current.id.clone(),
                name: current.name.clone(),
            }
        });
    }
    if previous.paired != current.paired {
        changes.push(DeviceChange::PairingChanged {
            id: current.id.clone(),
            name: current.name.clone(),
            paired: current.paired,
        });
    }
    if previous.battery != current.battery {
        changes.push(DeviceChange::BatteryChanged {
            id: current.id.clone(),
            name: current.name.clone(),
            battery: current.battery,
        });
    }
    changes.into_iter().map(StateChange::Device).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use handover_core::{Capability, MediaControl, PlaybackState};

    use super::*;

    fn device(connected: bool, percentage: Option<u8>) -> Device {
        Device {
            id: DeviceId::new("phone-123"),
            name: "Phone".into(),
            connected,
            paired: true,
            battery: percentage.map(|percentage| {
                BatteryState::new(percentage, false).expect("test battery is valid")
            }),
            capabilities: BTreeSet::from([Capability::Battery]),
        }
    }

    fn notification(device_id: &str, local_id: &str, title: &str) -> Notification {
        Notification {
            id: NotificationId::new(DeviceId::new(device_id), local_id),
            app_name: "Messages".into(),
            title: title.into(),
            body: "Hello".into(),
            icon_path: None,
            clearable: true,
            actions: vec![],
            reply_supported: true,
        }
    }

    fn media(device_id: &str, player_id: &str, title: &str) -> MediaSession {
        MediaSession {
            id: MediaSessionId::new(DeviceId::new(device_id), player_id),
            application: "Player".into(),
            title: Some(title.into()),
            artist: None,
            album: None,
            playback: PlaybackState::Playing,
            position_ms: Some(1000),
            duration_ms: Some(5000),
            volume_percent: Some(50),
            controls: std::collections::BTreeSet::from([MediaControl::Play, MediaControl::Seek]),
        }
    }

    #[test]
    fn media_sessions_are_scoped_updated_and_removed() {
        let mut store = StateStore::default();
        let first = media("phone-a", "Player", "First");
        let second = media("phone-b", "Player", "Second");
        assert!(
            store
                .apply(StateEvent::Media(MediaEvent::Added(first.clone())))
                .changed
        );
        assert!(
            store
                .apply(StateEvent::Media(MediaEvent::Added(second.clone())))
                .changed
        );
        assert!(
            !store
                .apply(StateEvent::Media(MediaEvent::Updated(first.clone())))
                .changed
        );
        let updated = media("phone-a", "Player", "Changed");
        assert!(
            store
                .apply(StateEvent::Media(MediaEvent::Updated(updated.clone())))
                .changed
        );
        assert!(
            store
                .apply(StateEvent::Media(MediaEvent::Removed(updated.id.clone())))
                .changed
        );
        assert!(
            !store
                .apply(StateEvent::Media(MediaEvent::Removed(updated.id)))
                .changed
        );
        assert_eq!(store.snapshot().media_sessions, vec![second]);
    }

    #[test]
    fn media_commands_require_current_session_device_and_capability() {
        let mut store = StateStore::default();
        let session = media("phone-123", "Player", "First");
        let play = MediaCommand::Play {
            id: session.id.clone(),
        };
        assert_eq!(
            store.validate_media_command(&play),
            Err(MediaValidationError::NotFound)
        );
        store.apply(StateEvent::Media(MediaEvent::Added(session.clone())));
        assert_eq!(
            store.validate_media_command(&play),
            Err(MediaValidationError::DeviceUnavailable)
        );
        let mut source = device(true, None);
        source.capabilities.insert(Capability::Media);
        store.apply(StateEvent::Device(DeviceEvent::Added(source)));
        assert!(store.validate_media_command(&play).is_ok());
        assert_eq!(
            store.validate_media_command(&MediaCommand::Pause {
                id: session.id.clone()
            }),
            Err(MediaValidationError::UnsupportedControl)
        );
        assert_eq!(
            store.validate_media_command(&MediaCommand::Seek {
                id: session.id.clone(),
                offset_ms: i64::MAX
            }),
            Err(MediaValidationError::InvalidValue)
        );
        let mut offline = device(false, None);
        offline.capabilities.insert(Capability::Media);
        store.apply(StateEvent::Device(DeviceEvent::Updated(offline)));
        assert_eq!(
            store.validate_media_command(&play),
            Err(MediaValidationError::DeviceUnavailable)
        );
    }

    #[test]
    fn media_rebuild_after_backend_restart_has_one_current_session() {
        let mut store = StateStore::default();
        let before = media("phone-a", "Player", "Old track");
        store.apply(StateEvent::Media(MediaEvent::Added(before.clone())));
        store.apply(StateEvent::Media(MediaEvent::Removed(before.id.clone())));
        let after = media("phone-a", "Player", "New track");
        store.apply(StateEvent::Media(MediaEvent::Added(after.clone())));
        assert_eq!(store.snapshot().media_sessions, vec![after]);
    }

    #[test]
    fn inserts_device() {
        let mut store = StateStore::default();
        let expected = device(false, None);

        let outcome = store.apply(StateEvent::Device(DeviceEvent::Added(expected.clone())));

        assert_eq!(store.get_device(&expected.id), Some(&expected));
        assert!(outcome.changed);
        assert_eq!(outcome.changes.len(), 1);
        assert!(matches!(
            outcome.changes[0],
            StateChange::Device(DeviceChange::Discovered { .. })
        ));
    }

    #[test]
    fn updates_device_and_reports_meaningful_changes() {
        let mut store = StateStore::default();
        store.apply(StateEvent::Device(DeviceEvent::Added(device(false, None))));
        let updated = device(true, Some(83));

        let outcome = store.apply(StateEvent::Device(DeviceEvent::Updated(updated.clone())));

        assert_eq!(store.get_device(&updated.id), Some(&updated));
        assert!(outcome.changed);
        assert_eq!(outcome.changes.len(), 2);
        assert!(matches!(
            outcome.changes[0],
            StateChange::Device(DeviceChange::Connected { .. })
        ));
        assert!(matches!(
            outcome.changes[1],
            StateChange::Device(DeviceChange::BatteryChanged { .. })
        ));
    }

    #[test]
    fn removes_device() {
        let mut store = StateStore::default();
        let known = device(false, None);
        store.apply(StateEvent::Device(DeviceEvent::Added(known.clone())));

        let outcome = store.apply(StateEvent::Device(DeviceEvent::Removed(known.id.clone())));

        assert_eq!(store.get_device(&known.id), None);
        assert!(outcome.changed);
        assert_eq!(outcome.changes.len(), 1);
        assert!(matches!(
            outcome.changes[0],
            StateChange::Device(DeviceChange::Removed { .. })
        ));
    }

    #[test]
    fn ignores_duplicate_state_and_unknown_removal() {
        let mut store = StateStore::default();
        let known = device(false, None);
        store.apply(StateEvent::Device(DeviceEvent::Added(known.clone())));

        let duplicate = store.apply(StateEvent::Device(DeviceEvent::Updated(known.clone())));
        let unknown = store.apply(StateEvent::Device(DeviceEvent::Removed(DeviceId::new(
            "unknown",
        ))));

        assert!(!duplicate.changed);
        assert!(duplicate.changes.is_empty());
        assert!(!unknown.changed);
        assert!(unknown.changes.is_empty());
    }

    #[test]
    fn notification_state_is_scoped_by_device_and_ignores_duplicates() {
        let mut store = StateStore::default();
        let first = notification("phone-a", "1", "Alice");
        let second = notification("phone-b", "1", "Bob");
        assert!(
            store
                .apply(StateEvent::Notification(NotificationEvent::Added(
                    first.clone()
                )))
                .changed
        );
        assert!(
            store
                .apply(StateEvent::Notification(NotificationEvent::Added(
                    second.clone()
                )))
                .changed
        );
        assert!(
            !store
                .apply(StateEvent::Notification(NotificationEvent::Updated(
                    first.clone()
                )))
                .changed
        );
        assert_eq!(store.snapshot().notifications.len(), 2);
        assert_eq!(store.get_notification(&first.id), Some(&first));

        let updated = notification("phone-a", "1", "Alice updated");
        let outcome = store.apply(StateEvent::Notification(NotificationEvent::Updated(
            updated.clone(),
        )));
        assert!(outcome.changed);
        assert!(matches!(
            outcome.changes[0],
            StateChange::Notification(NotificationChange::Updated(_))
        ));
        assert_eq!(store.get_notification(&updated.id), Some(&updated));

        assert!(
            store
                .apply(StateEvent::Notification(NotificationEvent::Removed(
                    updated.id.clone()
                )))
                .changed
        );
        assert_eq!(store.get_notification(&updated.id), None);
        assert_eq!(store.get_notification(&second.id), Some(&second));
        assert!(
            !store
                .apply(StateEvent::Notification(NotificationEvent::Removed(
                    updated.id
                )))
                .changed
        );
    }

    #[test]
    fn notification_commands_are_validated_against_current_state() {
        let mut store = StateStore::default();
        let known = notification("phone-a", "1", "Alice");
        let unknown = NotificationId::new(DeviceId::new("phone-a"), "missing");
        assert_eq!(
            store.validate_command(&NotificationCommand::Dismiss {
                notification_id: unknown
            }),
            Err(CommandValidationError::NotFound)
        );
        store.apply(StateEvent::Notification(NotificationEvent::Added(
            known.clone(),
        )));

        assert_eq!(
            store.validate_command(&NotificationCommand::InvokeAction {
                notification_id: known.id.clone(),
                action_id: "read".into()
            }),
            Err(CommandValidationError::UnknownAction)
        );
        assert_eq!(
            store.validate_command(&NotificationCommand::Reply {
                notification_id: known.id.clone(),
                text: "  ".into()
            }),
            Err(CommandValidationError::EmptyReply)
        );
        assert!(
            store
                .validate_command(&NotificationCommand::Reply {
                    notification_id: known.id.clone(),
                    text: "Hello".into()
                })
                .is_ok()
        );

        let mut restricted = known.clone();
        restricted.clearable = false;
        restricted.reply_supported = false;
        store.apply(StateEvent::Notification(NotificationEvent::Updated(
            restricted,
        )));
        assert_eq!(
            store.validate_command(&NotificationCommand::Dismiss {
                notification_id: known.id.clone()
            }),
            Err(CommandValidationError::NotClearable)
        );
        assert_eq!(
            store.validate_command(&NotificationCommand::Reply {
                notification_id: known.id,
                text: "Hello".into()
            }),
            Err(CommandValidationError::ReplyUnsupported)
        );
    }

    #[test]
    fn incoming_share_is_transient_and_does_not_enter_snapshot() {
        let mut store = StateStore::default();
        let share = ReceivedShare {
            device_id: DeviceId::new("phone-a"),
            resource: handover_core::SharedResource::Url {
                url: "https://example.com".into(),
            },
        };

        let outcome = store.apply(StateEvent::ShareReceived(share.clone()));

        assert!(outcome.changed);
        assert!(matches!(
            outcome.changes.as_slice(),
            [StateChange::ShareReceived(received)] if received == &share
        ));
        assert!(store.snapshot().devices.is_empty());
        assert!(store.snapshot().notifications.is_empty());
    }
}
