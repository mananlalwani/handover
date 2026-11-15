use crate::*;
use handover_core::{DeviceVolumeState, VolumeKind};
impl NativeBackend {
    pub fn volume_states(&self) -> Vec<DeviceVolumeState> {
        self.inner
            .lock()
            .unwrap()
            .volume_states
            .values()
            .cloned()
            .collect()
    }
    pub fn request_volume(&self, peer: &str) -> Result<(), NativeCommandError> {
        if !self
            .inner
            .lock()
            .unwrap()
            .peers
            .peers
            .get(peer)
            .is_some_and(|p| p.volume_control)
        {
            return Err(NativeCommandError::Unsupported);
        }
        self.queue_simple(
            peer,
            Message::VolumeRequest {
                protocol: WIRE_VERSION,
            },
        )
    }
    pub(crate) fn expire_volume_requests(
        &self,
        peer: &str,
        event: &Arc<dyn Fn(StateEvent) + Send + Sync>,
    ) {
        let expired = {
            let mut inner = self.inner.lock().unwrap();
            let pending = inner.volume_requests.entry(peer.into()).or_default();
            let expired: Vec<_> = pending
                .iter()
                .filter(|(_, time)| time.elapsed() >= Duration::from_secs(120))
                .map(|(id, _)| id.clone())
                .collect();
            for id in &expired {
                pending.remove(id);
            }
            expired
        };
        for request_id in expired {
            event(StateEvent::DeviceVolumeResult(
                handover_core::DeviceVolumeResult {
                    device_id: DeviceId::new(format!("native:{peer}")),
                    request_id,
                    accepted: false,
                    failure: Some(handover_core::DeviceVolumeFailure::Unconfirmed),
                },
            ));
        }
    }
    pub fn set_volume(
        &self,
        peer: &str,
        kind: VolumeKind,
        level: u32,
    ) -> Result<String, NativeCommandError> {
        let request_id = new_transfer_id().map_err(|_| NativeCommandError::QueueFull)?;
        {
            let mut inner = self.inner.lock().unwrap();
            if !inner.active.contains_key(peer) {
                return Err(NativeCommandError::Offline);
            }
            if !inner
                .peers
                .peers
                .get(peer)
                .is_some_and(|p| p.volume_control)
            {
                return Err(NativeCommandError::Unsupported);
            }
            let stream = inner
                .volume_states
                .get(peer)
                .and_then(|s| s.streams.iter().find(|s| s.kind == kind))
                .filter(|s| s.adjustable && level >= s.min_level && level <= s.max_level)
                .ok_or(NativeCommandError::InvalidResource)?;
            let _ = stream;
            let pending = inner.volume_requests.entry(peer.into()).or_default();
            if pending.len() >= 16 {
                return Err(NativeCommandError::QueueFull);
            }
            pending.insert(request_id.clone(), Instant::now());
        }
        if let Err(error) = self.queue_simple(
            peer,
            Message::VolumeSet {
                protocol: WIRE_VERSION,
                request_id: request_id.clone(),
                kind,
                level,
            },
        ) {
            self.inner
                .lock()
                .unwrap()
                .volume_requests
                .entry(peer.into())
                .or_default()
                .remove(&request_id);
            return Err(error);
        }
        Ok(request_id)
    }
}
