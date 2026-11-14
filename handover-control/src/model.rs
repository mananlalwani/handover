use handover_core::*;
use handover_ipc::{NativePeer, NativePendingPeer, ServerPayload};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Model {
    pub connected: bool,
    pub devices: BTreeMap<DeviceId, Device>,
    pub notifications: BTreeMap<NotificationId, Notification>,
    pub media: BTreeMap<MediaSessionId, MediaSession>,
    pub calls: BTreeMap<DeviceId, CallState>,
    pub contacts: Vec<Contact>,
    pub transfers: Vec<TransferActivity>,
    pub peers: Vec<NativePeer>,
    pub pairing: Vec<NativePendingPeer>,
    pub notice: String,
}
impl Model {
    pub fn apply(&mut self, payload: ServerPayload) {
        match payload {
            ServerPayload::Snapshot {
                devices,
                notifications,
                media_sessions,
                calls,
                ..
            } => {
                self.devices = devices.into_iter().map(|v| (v.id.clone(), v)).collect();
                self.notifications = notifications
                    .into_iter()
                    .map(|v| (v.id.clone(), v))
                    .collect();
                self.media = media_sessions
                    .into_iter()
                    .map(|v| (v.id.clone(), v))
                    .collect();
                self.calls = calls
                    .into_iter()
                    .map(|v| (v.device_id.clone(), v))
                    .collect();
            }
            ServerPayload::DeviceAdded { device } | ServerPayload::DeviceUpdated { device } => {
                self.devices.insert(device.id.clone(), device);
            }
            ServerPayload::DeviceRemoved { device_id } => {
                self.devices.remove(&device_id);
                self.notifications.retain(|id, _| id.device_id != device_id);
                self.contacts.retain(|c| c.device_id != device_id);
                self.media.retain(|id, _| id.device_id != device_id);
                self.calls.remove(&device_id);
            }
            ServerPayload::NotificationAdded { notification }
            | ServerPayload::NotificationUpdated { notification } => {
                self.notifications
                    .insert(notification.id.clone(), notification);
            }
            ServerPayload::NotificationRemoved { notification_id } => {
                self.notifications.remove(&notification_id);
            }
            ServerPayload::MediaAdded { media_session }
            | ServerPayload::MediaUpdated { media_session } => {
                self.media.insert(media_session.id.clone(), media_session);
            }
            ServerPayload::MediaRemoved { media_session_id } => {
                self.media.remove(&media_session_id);
            }
            ServerPayload::CallUpdated { call } => {
                self.calls.insert(call.device_id.clone(), call);
            }
            ServerPayload::CallRemoved { device_id } => {
                self.calls.remove(&device_id);
            }
            ServerPayload::ContactsRemoved { device_id } => {
                self.contacts.retain(|c| c.device_id != device_id)
            }
            ServerPayload::ContactsSyncFailed { .. } => {
                self.notice = "Contact sync failed. The previous address book is kept.".into()
            }
            ServerPayload::DeviceCommandResult { .. } | ServerPayload::CallCommandResult { .. } => {
            }
            _ => {}
        }
    }
    pub fn ready(&self, id: &DeviceId) -> bool {
        self.connected
            && self
                .devices
                .get(id)
                .is_some_and(|d| d.connected && d.paired)
    }
    pub fn name(&self, id: &DeviceId) -> String {
        self.devices
            .get(id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| "Device".into())
    }
}
