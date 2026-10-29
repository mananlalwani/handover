use std::collections::{BTreeMap, BTreeSet};

use handover_core::{Notification, NotificationCommand, NotificationId};

/// Bounds both visible notifications and remembered local dismissals.
pub(crate) const MAX_NOTIFICATIONS: usize = 512;
const DISMISS: &str = "dismiss";
const ACTION_PREFIX: &str = "action:";

#[derive(Default)]
pub(crate) struct Model {
    pub entries: BTreeMap<NotificationId, Entry>,
}

pub(crate) struct Entry {
    pub notification: Notification,
    pub desktop_id: Option<u32>,
}

impl Model {
    /// Reconcile authoritative state, without creating alerts for historical items.
    pub fn snapshot(&mut self, notifications: Vec<Notification>) -> Vec<u32> {
        let current: BTreeSet<_> = notifications.iter().map(|item| item.id.clone()).collect();
        let mut close = Vec::new();
        self.entries.retain(|id, entry| {
            if current.contains(id) {
                true
            } else {
                close.extend(entry.desktop_id);
                false
            }
        });
        for notification in notifications {
            if let Some(entry) = self.entries.get_mut(&notification.id) {
                // Visible updates are handled by the caller before storing the snapshot.
                entry.notification = notification;
            } else if self.entries.len() < MAX_NOTIFICATIONS {
                self.entries.insert(
                    notification.id.clone(),
                    Entry {
                        notification,
                        desktop_id: None,
                    },
                );
            }
        }
        close
    }

    pub fn remove(&mut self, id: &NotificationId) -> Option<u32> {
        self.entries.remove(id).and_then(|entry| entry.desktop_id)
    }

    pub fn closed(&mut self, desktop_id: u32) {
        for entry in self.entries.values_mut() {
            if entry.desktop_id == Some(desktop_id) {
                entry.desktop_id = None;
            }
        }
    }

    pub fn action(&self, desktop_id: u32, key: &str) -> Option<NotificationCommand> {
        let notification = &self
            .entries
            .values()
            .find(|entry| entry.desktop_id == Some(desktop_id))?
            .notification;
        if key == DISMISS && notification.clearable {
            return Some(NotificationCommand::Dismiss {
                notification_id: notification.id.clone(),
            });
        }
        let action_id = key.strip_prefix(ACTION_PREFIX)?;
        notification
            .actions
            .iter()
            .any(|action| action.id == action_id)
            .then(|| NotificationCommand::InvokeAction {
                notification_id: notification.id.clone(),
                action_id: action_id.to_owned(),
            })
    }
}

pub(crate) fn actions(notification: &Notification) -> Vec<String> {
    let mut actions = Vec::new();
    for action in &notification.actions {
        actions.push(format!("{ACTION_PREFIX}{}", action.id));
        actions.push(action.label.clone());
    }
    if notification.clearable {
        actions.extend([DISMISS.to_owned(), "Dismiss on phone".to_owned()]);
    }
    actions
}

pub(crate) fn body(text: &str, markup: bool) -> String {
    if !markup {
        return text.to_owned();
    }
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
