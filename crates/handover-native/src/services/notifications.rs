use std::collections::BTreeSet;
use std::sync::Arc;

use handover_core::*;

use crate::limits::*;
use crate::protocol::*;
use crate::{NativeBackend, NativeError, Peer, device};

pub(crate) fn normalize_native_notification(
    peer: &Peer,
    wire: &WireNotification,
) -> Result<Notification, NativeError> {
    let key = wire.key.trim();
    if key.is_empty() || key.len() > MAX_NOTIFICATION_KEY || key.chars().any(char::is_control) {
        return Err(NativeError::InvalidFrame);
    }
    if wire.app.len() > MAX_NOTIFICATION_APP
        || wire.title.len() > MAX_NOTIFICATION_TITLE
        || wire.body.len() > MAX_NOTIFICATION_BODY
    {
        return Err(NativeError::InvalidFrame);
    }
    if wire.actions.len() > MAX_NOTIFICATION_ACTIONS {
        return Err(NativeError::InvalidFrame);
    }
    let mut seen = BTreeSet::new();
    let mut actions = Vec::with_capacity(wire.actions.len());
    for action in &wire.actions {
        if action.id.is_empty()
            || action.id.len() > MAX_NOTIFICATION_ACTION_ID
            || action.label.is_empty()
            || action.label.len() > MAX_NOTIFICATION_ACTION_LABEL
            || !seen.insert(action.id.clone())
        {
            return Err(NativeError::InvalidFrame);
        }
        actions.push(NotificationAction {
            id: action.id.clone(),
            label: action.label.clone(),
        });
    }
    Ok(Notification {
        id: NotificationId::new(DeviceId::new(format!("native:{}", peer.id)), key.to_owned()),
        app_name: wire.app.clone(),
        title: wire.title.clone(),
        body: wire.body.clone(),
        icon_path: None,
        clearable: wire.clearable,
        actions,
        reply_supported: wire.reply_supported,
    })
}

/// Handle phone notifications without logging their content.
impl NativeBackend {
    pub(crate) fn handle_notification_post(
        &self,
        peer: &Peer,
        wire: WireNotification,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        let notification = normalize_native_notification(peer, &wire)?;
        let mut inner = self.inner.lock().unwrap();
        if !inner.peers.peers.contains_key(&peer.id) {
            return Err(NativeError::InvalidFrame);
        }
        let keys = inner.notif_keys.entry(peer.id.clone()).or_default();
        let is_new = !keys.contains(&wire.key);
        keys.insert(wire.key.clone());
        let device_changed = inner.notif_enabled.get(&peer.id).copied() != Some(true);
        inner.notif_enabled.insert(peer.id.clone(), true);
        let device_update = device_changed.then(|| {
            device(
                peer,
                true,
                inner.batteries.get(&peer.id).cloned(),
                true,
                inner.media_enabled.get(&peer.id).copied().unwrap_or(false),
            )
        });
        drop(inner);
        if let Some(device) = device_update {
            event(StateEvent::Device(DeviceEvent::Updated(device)));
        }
        event(StateEvent::Notification(if is_new {
            NotificationEvent::Added(notification)
        } else {
            NotificationEvent::Updated(notification)
        }));
        Ok(())
    }

    pub(crate) fn handle_notification_removed(
        &self,
        peer: &Peer,
        key: &str,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        if key.is_empty() || key.len() > MAX_NOTIFICATION_KEY {
            return Err(NativeError::InvalidFrame);
        }
        let mut inner = self.inner.lock().unwrap();
        let known = inner
            .notif_keys
            .get_mut(&peer.id)
            .is_some_and(|keys| keys.remove(key));
        drop(inner);
        if known {
            event(StateEvent::Notification(NotificationEvent::Removed(
                NotificationId::new(DeviceId::new(format!("native:{}", peer.id)), key.to_owned()),
            )));
        }
        Ok(())
    }

    pub(crate) fn handle_notifications_sync(
        &self,
        peer: &Peer,
        enabled: bool,
        wires: Vec<WireNotification>,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) -> Result<(), NativeError> {
        if wires.len() > MAX_NOTIFICATIONS_PER_SYNC {
            return Err(NativeError::InvalidFrame);
        }
        let mut notifications = Vec::with_capacity(wires.len());
        let mut keys = BTreeSet::new();
        for wire in &wires {
            if !keys.insert(wire.key.clone()) {
                return Err(NativeError::InvalidFrame);
            }
            notifications.push(normalize_native_notification(peer, wire)?);
        }
        if !enabled && !notifications.is_empty() {
            return Err(NativeError::InvalidFrame);
        }
        let mut inner = self.inner.lock().unwrap();
        if !inner.peers.peers.contains_key(&peer.id) {
            return Err(NativeError::InvalidFrame);
        }
        let previous = inner
            .notif_keys
            .insert(peer.id.clone(), keys.clone())
            .unwrap_or_default();
        let device_changed = inner.notif_enabled.get(&peer.id).copied() != Some(enabled);
        inner.notif_enabled.insert(peer.id.clone(), enabled);
        let device_update = device_changed.then(|| {
            device(
                peer,
                true,
                inner.batteries.get(&peer.id).cloned(),
                enabled,
                inner.media_enabled.get(&peer.id).copied().unwrap_or(false),
            )
        });
        drop(inner);
        if let Some(device) = device_update {
            event(StateEvent::Device(DeviceEvent::Updated(device)));
        }
        for id in previous.difference(&keys) {
            event(StateEvent::Notification(NotificationEvent::Removed(
                NotificationId::new(DeviceId::new(format!("native:{}", peer.id)), id.clone()),
            )));
        }
        for notification in notifications {
            let id = notification.id.clone();
            let is_new = !previous.contains(&id.local_id);
            event(StateEvent::Notification(if is_new {
                NotificationEvent::Added(notification)
            } else {
                NotificationEvent::Updated(notification)
            }));
        }
        if !enabled {
            // Media observation shares the notification-listener permission,
            // so a revoked listener must not leave native sessions behind.
            self.clear_native_media(peer, event);
        }
        Ok(())
    }
}
