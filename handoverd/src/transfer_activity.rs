use handover_core::{DeviceId, FileDownloadState, TransferActivity, TransferActivityEvent};
use std::{
    collections::VecDeque,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Default)]
pub(crate) struct ActivityLog {
    sequence: u64,
    entries: VecDeque<TransferActivity>,
}

impl ActivityLog {
    pub(crate) fn snapshot(&self) -> Vec<TransferActivity> {
        self.entries.iter().cloned().collect()
    }
    pub(crate) fn remove_device(&mut self, device: &DeviceId) {
        self.entries
            .retain(|entry| device_id(&entry.event) != device);
    }
    pub(crate) fn record(&mut self, event: TransferActivityEvent) {
        if let TransferActivityEvent::Download { download } = &event {
            let key = (&download.device_id, download.transfer_id.as_str());
            if let Some(old) = self
                .entries
                .iter()
                .find(|old| download_key(&old.event) == Some(key))
            {
                if old.event == event {
                    return;
                }
                if let TransferActivityEvent::Download { download: previous } = &old.event {
                    if previous.state.is_terminal() {
                        return;
                    }
                    if matches!(previous.state, FileDownloadState::Receiving)
                        && matches!(download.state, FileDownloadState::Queued)
                    {
                        return;
                    }
                }
            }
            self.entries
                .retain(|old| download_key(&old.event) != Some(key));
        }
        if let Some((device, id)) = outgoing(&event) {
            if matches!(event, TransferActivityEvent::Progress { .. })
                && self.entries.iter().any(|old| {
                    matches!(old.event, TransferActivityEvent::Result { .. })
                        && outgoing(&old.event) == Some((device, id))
                })
            {
                return;
            }
            self.entries
                .retain(|old| outgoing(&old.event) != Some((device, id)));
        }
        let Some(sequence) = self.sequence.checked_add(1) else {
            return;
        };
        self.sequence = sequence;
        self.entries.push_front(TransferActivity {
            sequence,
            observed_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
            event,
        });
        while self.entries.len() > 128 {
            let removable = self.entries.iter().rposition(|entry| match &entry.event {
                TransferActivityEvent::Download { download } => download.state.is_terminal(),
                _ => true,
            });
            if let Some(index) = removable {
                self.entries.remove(index);
            } else {
                self.entries.pop_back();
            }
        }
    }
}
fn device_id(event: &TransferActivityEvent) -> &DeviceId {
    match event {
        TransferActivityEvent::Received { share } => &share.device_id,
        TransferActivityEvent::Progress { progress } => &progress.device_id,
        TransferActivityEvent::Result { result } => &result.device_id,
        TransferActivityEvent::Download { download } => &download.device_id,
    }
}
fn outgoing(event: &TransferActivityEvent) -> Option<(&DeviceId, &str)> {
    match event {
        TransferActivityEvent::Received { .. } => None,
        TransferActivityEvent::Progress { progress } => {
            Some((&progress.device_id, &progress.transfer_id))
        }
        TransferActivityEvent::Result { result } => Some((&result.device_id, &result.transfer_id)),
        TransferActivityEvent::Download { .. } => None,
    }
}
fn download_key(event: &TransferActivityEvent) -> Option<(&DeviceId, &str)> {
    match event {
        TransferActivityEvent::Download { download } => {
            Some((&download.device_id, &download.transfer_id))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use handover_core::{FileDownload, FileDownloadState, ShareProgress, ShareResult, ShareStatus};

    #[test]
    fn file_download_status_coalesces_and_never_regresses() {
        let mut log = ActivityLog::default();
        let device_id = DeviceId::new("native:phone");
        let make = |state| TransferActivityEvent::Download {
            download: FileDownload {
                device_id: device_id.clone(),
                transfer_id: "download-1".into(),
                source_path: "DCIM/photo.jpg".into(),
                state,
            },
        };
        log.record(make(FileDownloadState::Queued));
        log.record(make(FileDownloadState::Receiving));
        log.record(make(FileDownloadState::Queued));
        assert!(
            matches!(&log.snapshot()[0].event, TransferActivityEvent::Download { download } if matches!(download.state, FileDownloadState::Receiving))
        );
        log.record(make(FileDownloadState::Completed {
            path: "/home/user/Downloads/photo.jpg".into(),
        }));
        log.record(make(FileDownloadState::Receiving));
        assert_eq!(log.snapshot().len(), 1);
        assert!(
            matches!(&log.snapshot()[0].event, TransferActivityEvent::Download { download } if matches!(download.state, FileDownloadState::Completed { .. }))
        );
    }
    #[test]
    fn progress_is_coalesced_terminal_evidence_wins_and_history_is_bounded() {
        let mut log = ActivityLog::default();
        let device = DeviceId::new("fixture");
        let progress = ShareProgress {
            device_id: device.clone(),
            transfer_id: "transfer".into(),
            bytes_sent: 5,
            total_bytes: 10,
        };
        log.record(TransferActivityEvent::Progress {
            progress: progress.clone(),
        });
        log.record(TransferActivityEvent::Result {
            result: ShareResult {
                device_id: device.clone(),
                transfer_id: "transfer".into(),
                status: ShareStatus::Completed,
                reason: None,
            },
        });
        log.record(TransferActivityEvent::Progress { progress });
        assert_eq!(log.snapshot().len(), 1);
        assert!(matches!(
            log.snapshot()[0].event,
            TransferActivityEvent::Result { .. }
        ));
        for n in 0..200 {
            log.record(TransferActivityEvent::Result {
                result: ShareResult {
                    device_id: device.clone(),
                    transfer_id: n.to_string(),
                    status: ShareStatus::Failed,
                    reason: None,
                },
            });
        }
        assert_eq!(log.snapshot().len(), 128);
        log.remove_device(&device);
        assert!(log.snapshot().is_empty());
    }
}
