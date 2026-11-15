use super::*;

pub(crate) fn native_command_response(
    result: Option<Result<(), handover_native::NativeCommandError>>,
    operation: &str,
) -> ServerPayload {
    match result {
        Some(Ok(())) => ServerPayload::NativeAccepted,
        Some(Err(error)) => ServerPayload::Error {
            code: match error {
                handover_native::NativeCommandError::Offline => ErrorCode::DeviceDisconnected,
                handover_native::NativeCommandError::QueueFull => ErrorCode::BackendRejected,
                handover_native::NativeCommandError::Unsupported => ErrorCode::BackendRejected,
                handover_native::NativeCommandError::InvalidResource => ErrorCode::MalformedRequest,
            },
            message: format!("{operation} was not accepted"),
            conversation_id: None,
        },
        None => ServerPayload::Error {
            code: ErrorCode::BackendUnavailable,
            message: "native backend unavailable".into(),
            conversation_id: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn audio_subscription_is_opt_in_and_starts_with_current_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("audio.sock");
        let state = Arc::new(RwLock::new(StateStore::default()));
        let (events, _) = broadcast::channel(16);
        let server = IpcServer::bind_at(path.clone(), state, events.clone(), None)
            .await
            .unwrap();
        let task = tokio::spawn(async move {
            server.run().await.unwrap();
        });
        let mut legacy = handover_ipc::Client::connect_to(path.clone())
            .await
            .unwrap()
            .subscribe()
            .await
            .unwrap();
        let mut audio = handover_ipc::Client::connect_to(path)
            .await
            .unwrap()
            .subscribe_with_audio()
            .await
            .unwrap();
        assert!(matches!(
            audio.next_message().await.unwrap().payload,
            ServerPayload::AudioStreams { .. }
        ));
        let status = handover_core::AudioStreamStatus {
            device_id: DeviceId::new("phone"),
            state: handover_core::AudioStreamPhase::Streaming,
            ..handover_core::AudioStreamStatus::idle(DeviceId::new("phone"))
        };
        events
            .send(StateEvent::AudioStream(status.clone()))
            .unwrap();
        assert_eq!(
            audio.next_message().await.unwrap().payload,
            ServerPayload::AudioStreamUpdated { status }
        );
        events
            .send(StateEvent::Device(DeviceEvent::Removed(DeviceId::new(
                "phone",
            ))))
            .unwrap();
        assert!(matches!(
            legacy.next_message().await.unwrap().payload,
            ServerPayload::DeviceRemoved { .. }
        ));
        task.abort();
    }
    #[tokio::test]
    async fn phone_audio_events_require_opt_in_and_preserve_legacy_subscriptions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("phone-audio.sock");
        let state = Arc::new(RwLock::new(StateStore::default()));
        let (events, _) = broadcast::channel(16);
        let server = IpcServer::bind_at(path.clone(), state, events.clone(), None)
            .await
            .unwrap();
        let task = tokio::spawn(async move {
            server.run().await.unwrap();
        });
        let mut legacy = handover_ipc::Client::connect_to(path.clone())
            .await
            .unwrap()
            .subscribe()
            .await
            .unwrap();
        let mut control = handover_ipc::Client::connect_to(path)
            .await
            .unwrap()
            .subscribe_control_center_with_audio()
            .await
            .unwrap();
        assert!(matches!(
            control.next_message().await.unwrap().payload,
            ServerPayload::AudioStreams { .. }
        ));
        assert!(matches!(
            control.next_message().await.unwrap().payload,
            ServerPayload::CallAudioControls { .. }
        ));
        let volume = handover_core::DeviceVolumeState {
            device_id: DeviceId::new("native:fixture"),
            streams: Vec::new(),
        };
        events
            .send(StateEvent::DeviceVolume(volume.clone()))
            .unwrap();
        assert_eq!(
            control.next_message().await.unwrap().payload,
            ServerPayload::DeviceVolume { state: volume }
        );
        events
            .send(StateEvent::CallAudioControls(Default::default()))
            .unwrap();
        assert!(matches!(
            control.next_message().await.unwrap().payload,
            ServerPayload::CallAudioControls { .. }
        ));
        events
            .send(StateEvent::DeviceVolumeResult(
                handover_core::DeviceVolumeResult {
                    device_id: DeviceId::new("native:fixture"),
                    request_id: "fixture".into(),
                    accepted: false,
                    failure: Some(handover_core::DeviceVolumeFailure::Unconfirmed),
                },
            ))
            .unwrap();
        assert!(matches!(
            control.next_message().await.unwrap().payload,
            ServerPayload::DeviceVolumeResult { .. }
        ));
        events
            .send(StateEvent::Device(DeviceEvent::Removed(DeviceId::new(
                "native:fixture",
            ))))
            .unwrap();
        assert!(matches!(
            legacy.next_message().await.unwrap().payload,
            ServerPayload::DeviceRemoved { .. }
        ));
        task.abort();
    }
}
