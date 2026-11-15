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
    pub call_audio: Option<CallAudioStatus>,
    pub audio_controls: Option<AudioControlsSnapshot>,
    pub audio_error: Option<String>,
    pub audio_streams: BTreeMap<DeviceId, AudioStreamStatus>,
    pub commands: Option<Vec<CustomCommandEntry>>,
    pub command_result: Option<CustomCommandResult>,
    pub contacts: Vec<Contact>,
    pub folder: Option<FilesystemResult>,
    pub clipboard_mirror: Option<bool>,
    pub clipboard_history: Option<Vec<handover_ipc::ClipboardHistoryEntry>>,
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
                self.audio_streams
                    .retain(|id, _| self.devices.contains_key(id));
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
                self.audio_streams.remove(&device_id);
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
            ServerPayload::AudioStreams { streams } => {
                for stream in streams {
                    self.observe_audio_stream(stream);
                }
            }
            ServerPayload::AudioStreamUpdated { status } => self.observe_audio_stream(status),
            ServerPayload::AudioControlsUpdated { snapshot }
            | ServerPayload::AudioControls { snapshot } => self.observe_audio_controls(snapshot),
            ServerPayload::ContactsRemoved { device_id } => {
                self.contacts.retain(|c| c.device_id != device_id)
            }
            ServerPayload::ContactsSyncFailed { .. } => {
                self.notice = "Contact sync failed. The previous address book is kept.".into()
            }
            ServerPayload::DeviceCommandResult { result } => {
                self.notice = format!(
                    "{}: {} request {}{}",
                    self.name(&result.device_id),
                    match result.action {
                        DeviceCommandAction::Ping => "ping",
                        DeviceCommandAction::Ring => "ring",
                        DeviceCommandAction::Lock => "lock",
                        DeviceCommandAction::Clipboard => "clipboard",
                        DeviceCommandAction::Notification => "notification",
                        DeviceCommandAction::Media => "media",
                        DeviceCommandAction::Screensaver => "desktop sleep",
                        DeviceCommandAction::KeepAwake => "awake mode",
                        DeviceCommandAction::Tethering => "hotspot settings",
                    },
                    if result.accepted {
                        "accepted by phone"
                    } else {
                        "rejected by phone"
                    },
                    result
                        .failure
                        .map(|f| match f {
                            DeviceCommandFailure::PermissionDenied =>
                                " · Permission denied".to_owned(),
                            DeviceCommandFailure::Unavailable => " · Unavailable".to_owned(),
                            DeviceCommandFailure::Rejected => " · Rejected".to_owned(),
                        })
                        .unwrap_or_default()
                );
            }
            ServerPayload::CallCommandResult { result } => {
                self.notice = format!(
                    "{}: call {} {}{}",
                    self.name(&result.device_id),
                    result.action.as_str(),
                    if result.accepted {
                        "accepted by phone"
                    } else {
                        "rejected by phone"
                    },
                    result
                        .failure
                        .map(|f| match f {
                            CallCommandFailure::PermissionDenied =>
                                " · Permission denied".to_owned(),
                            CallCommandFailure::StaleState => " · Call changed".to_owned(),
                            CallCommandFailure::EmergencyNumber =>
                                " · Use the phone to dial this number".to_owned(),
                            CallCommandFailure::InvalidAddress =>
                                " · Invalid phone number".to_owned(),
                            CallCommandFailure::WrongPhase =>
                                " · Action unavailable in this call".to_owned(),
                            CallCommandFailure::Unsupported => " · Unsupported".to_owned(),
                            CallCommandFailure::Rejected => " · Rejected".to_owned(),
                        })
                        .unwrap_or_default()
                );
            }
            ServerPayload::Filesystem { result } => {
                self.folder = Some(result);
            }
            _ => {}
        }
    }
    pub fn observe_audio_controls(&mut self, snapshot: AudioControlsSnapshot) {
        if self
            .audio_controls
            .as_ref()
            .is_none_or(|current| snapshot.revision >= current.revision)
        {
            self.audio_controls = Some(snapshot);
            self.audio_error = None;
        }
    }
    pub fn observe_audio_stream(&mut self, stream: AudioStreamStatus) {
        if self.devices.contains_key(&stream.device_id)
            && self
                .audio_streams
                .get(&stream.device_id)
                .is_none_or(|current| stream.revision >= current.revision)
        {
            self.audio_streams.insert(stream.device_id.clone(), stream);
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
