//! Requested incoming files use the same authenticated stream and atomic receiver as shares.
use crate::*;
use handover_core::{FileDownload, FileDownloadFailure, FileDownloadState};

pub(crate) struct PendingDownload {
    pub(crate) download: FileDownload,
    pub(crate) started: Instant,
}

impl NativeBackend {
    /// Queue a request. Only the receiver's atomic save can report completion.
    pub fn request_file(
        &self,
        peer_id: &str,
        path: String,
    ) -> Result<FileDownload, NativeCommandError> {
        if path == "."
            || !safe_browse_path(&path)
            || path
                .split('/')
                .any(|part| part.len() > 255 || part.chars().any(char::is_control))
        {
            return Err(NativeCommandError::InvalidResource);
        }
        let mut inner = self.inner.lock().unwrap();
        if !inner.active.contains_key(peer_id) {
            return Err(NativeCommandError::Offline);
        }
        if !inner
            .peers
            .peers
            .get(peer_id)
            .is_some_and(|peer| peer.file_downloads)
        {
            return Err(NativeCommandError::Unsupported);
        }
        let active: usize = inner.pending_downloads.values().map(BTreeMap::len).sum();
        let pending = inner.pending_downloads.entry(peer_id.into()).or_default();
        if active >= 32
            || pending.len() >= 8
            || pending.values().any(|p| p.download.source_path == path)
            || inner
                .outbox
                .get(peer_id)
                .is_some_and(|q| q.len() >= MAX_OUTBOX_PER_PEER)
        {
            return Err(NativeCommandError::QueueFull);
        }
        let transfer_id = new_transfer_id().map_err(|_| NativeCommandError::QueueFull)?;
        let download = FileDownload {
            device_id: DeviceId::new(format!("native:{peer_id}")),
            transfer_id: transfer_id.clone(),
            source_path: path.clone(),
            state: FileDownloadState::Queued,
        };
        inner
            .pending_downloads
            .entry(peer_id.into())
            .or_default()
            .insert(
                transfer_id.clone(),
                PendingDownload {
                    download: download.clone(),
                    started: Instant::now(),
                },
            );
        inner
            .outbox
            .entry(peer_id.into())
            .or_default()
            .push(Message::FileRequest {
                protocol: WIRE_VERSION,
                transfer_id,
                path,
            });
        Ok(download)
    }

    pub(crate) fn download_started(
        &self,
        peer: &str,
        id: &str,
        name: &str,
    ) -> Result<Option<FileDownload>, NativeError> {
        let mut inner = self.inner.lock().unwrap();
        let Some(pending) = inner
            .pending_downloads
            .get_mut(peer)
            .and_then(|p| p.get_mut(id))
        else {
            return Ok(None);
        };
        if pending.download.source_path.rsplit('/').next() != Some(name)
            || pending.started.elapsed() >= SHARE_RESULT_TIMEOUT
        {
            return Err(NativeError::InvalidFrame);
        }
        pending.download.state = FileDownloadState::Receiving;
        Ok(Some(pending.download.clone()))
    }

    pub(crate) fn finish_download(
        &self,
        peer: &str,
        id: &str,
        state: FileDownloadState,
    ) -> Option<FileDownload> {
        let mut inner = self.inner.lock().unwrap();
        let mut pending = inner.pending_downloads.get_mut(peer)?.remove(id)?;
        pending.download.state = state;
        Some(pending.download)
    }

    pub(crate) fn expire_downloads(&self, peer: &str, now: Instant) -> Vec<FileDownload> {
        let mut inner = self.inner.lock().unwrap();
        let pending = inner.pending_downloads.entry(peer.into()).or_default();
        let expired: Vec<_> = pending
            .iter()
            .filter(|(_, p)| now.saturating_duration_since(p.started) >= SHARE_RESULT_TIMEOUT)
            .map(|(id, _)| id.clone())
            .collect();
        let downloads = expired
            .iter()
            .filter_map(|id| pending.remove(id))
            .map(|mut p| {
                p.download.state = FileDownloadState::Failed {
                    reason: FileDownloadFailure::TimedOut,
                };
                p.download
            })
            .collect();
        if let Some(queue) = inner.outbox.get_mut(peer) {
            queue.retain(|message| !matches!(message, Message::FileRequest {transfer_id,..} if expired.contains(transfer_id)));
        }
        downloads
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timeout_removes_unsent_request_and_emits_one_terminal_record() {
        let directory = tempfile::tempdir().unwrap();
        let backend = NativeBackend::open(directory.path().into()).unwrap();
        let now = Instant::now();
        let id = "0123456789abcdef0123456789abcdef";
        let download = FileDownload {
            device_id: DeviceId::new("native:phone"),
            transfer_id: id.into(),
            source_path: "Code/untouched.cc".into(),
            state: FileDownloadState::Queued,
        };
        {
            let mut inner = backend.inner.lock().unwrap();
            inner
                .pending_downloads
                .entry("phone".into())
                .or_default()
                .insert(
                    id.into(),
                    PendingDownload {
                        download: download.clone(),
                        started: now - SHARE_RESULT_TIMEOUT,
                    },
                );
            inner.outbox.insert(
                "phone".into(),
                vec![
                    Message::FileRequest {
                        protocol: WIRE_VERSION,
                        transfer_id: id.into(),
                        path: download.source_path.clone(),
                    },
                    Message::Ping {
                        protocol: WIRE_VERSION,
                    },
                ],
            );
        }
        let expired = backend.expire_downloads("phone", now);
        assert_eq!(expired.len(), 1);
        assert_eq!(
            expired[0].state,
            FileDownloadState::Failed {
                reason: FileDownloadFailure::TimedOut
            }
        );
        assert_eq!(expired[0].source_path, download.source_path);
        assert!(backend.expire_downloads("phone", now).is_empty());
        assert!(matches!(
            backend.inner.lock().unwrap().outbox["phone"].as_slice(),
            [Message::Ping { .. }]
        ));
    }
}
