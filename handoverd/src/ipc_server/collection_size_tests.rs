use super::*;

use handover_core::{
    DeviceId, MediaSession, MediaSessionId, Notification, NotificationAction, NotificationId,
    PlaybackState,
};
use tokio::io::AsyncReadExt;

fn notification(index: usize) -> Notification {
    Notification {
        id: NotificationId::new(DeviceId::new("phone"), index.to_string()),
        app_name: "Messages".into(),
        title: format!("Notification {index}"),
        body: "x".repeat(4096),
        icon_path: None,
        clearable: true,
        actions: vec![NotificationAction {
            id: "open".into(),
            label: "Open".into(),
        }],
        reply_supported: false,
    }
}

fn media_session(index: usize) -> MediaSession {
    let metadata = "x".repeat(512);
    MediaSession {
        id: MediaSessionId::new(DeviceId::new("phone"), format!("player-{index}")),
        application: "Music".into(),
        title: Some(metadata.clone()),
        artist: Some(metadata.clone()),
        album: Some(metadata),
        playback: PlaybackState::Paused,
        position_ms: None,
        duration_ms: None,
        volume_percent: None,
        controls: Default::default(),
    }
}

fn split_lines(bytes: Vec<u8>) -> Vec<Vec<u8>> {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

#[tokio::test]
async fn large_notification_response_chunks_at_preferred_size_and_reconstructs() {
    let expected: Vec<_> = (0..400).map(notification).collect();
    let (mut writer, mut reader) = tokio::io::duplex(4 * handover_ipc::MAX_LINE_BYTES);
    write_notifications_response(&mut writer, expected.clone())
        .await
        .expect("write collection response");
    drop(writer);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .await
        .expect("read response bytes");
    let lines = split_lines(bytes);
    assert!(lines.len() > 1);
    let mut actual = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        assert!(line.len() < handover_ipc::PREFERRED_LINE_BYTES);
        let message: ServerMessage = serde_json::from_slice(line).expect("valid response");
        match message.payload {
            ServerPayload::NotificationsChunk {
                notifications,
                done,
            } => {
                assert_eq!(done, index + 1 == lines.len());
                actual.extend(notifications);
            }
            payload => panic!("large list should use chunks, got {payload:?}"),
        }
    }
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn large_media_response_chunks_at_preferred_size_and_reconstructs() {
    let expected: Vec<_> = (0..700).map(media_session).collect();
    let (mut writer, mut reader) = tokio::io::duplex(4 * handover_ipc::MAX_LINE_BYTES);
    write_media_response(&mut writer, expected.clone())
        .await
        .expect("write collection response");
    drop(writer);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .await
        .expect("read response bytes");
    let lines = split_lines(bytes);
    assert!(lines.len() > 1);
    let mut actual = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        assert!(line.len() < handover_ipc::PREFERRED_LINE_BYTES);
        let message: ServerMessage = serde_json::from_slice(line).expect("valid response");
        match message.payload {
            ServerPayload::MediaChunk {
                media_sessions,
                done,
            } => {
                assert_eq!(done, index + 1 == lines.len());
                actual.extend(media_sessions);
            }
            payload => panic!("large list should use chunks, got {payload:?}"),
        }
    }
    assert_eq!(actual, expected);
}
