use super::*;

use handover_core::{Contact, DeviceId, MessagingAccountId, NotificationAction};
use tokio::io::BufReader;
use tokio::net::unix::OwnedReadHalf;
use tokio::time::{Duration, timeout};

fn notification(body: &str) -> Notification {
    Notification {
        id: NotificationId::new(DeviceId::new("phone"), "notice"),
        app_name: "Messages".into(),
        title: "Title".into(),
        body: body.into(),
        icon_path: None,
        clearable: true,
        actions: vec![NotificationAction {
            id: "open".into(),
            label: "Open".into(),
        }],
        reply_supported: false,
    }
}

async fn client_pair() -> (Client, BufReader<OwnedReadHalf>, OwnedWriteHalf) {
    let (client_stream, server_stream) = UnixStream::pair().expect("pair sockets");
    let (reader, writer) = client_stream.into_split();
    let (server_reader, server_writer) = server_stream.into_split();
    (
        Client {
            reader: BufReader::new(reader),
            writer,
        },
        BufReader::new(server_reader),
        server_writer,
    )
}

async fn read_request(reader: &mut BufReader<OwnedReadHalf>) -> Request {
    read_json_line(reader)
        .await
        .expect("read request")
        .expect("request exists")
}

#[tokio::test]
async fn contacts_reject_more_than_256_chunks() {
    let (mut client, mut server_reader, mut server_writer) = client_pair().await;
    let server = tokio::spawn(async move {
        assert_eq!(
            read_request(&mut server_reader).await.method,
            Method::ContactsList
        );
        for index in 0..=MAX_SNAPSHOT_CHUNKS {
            write_json_line(
                &mut server_writer,
                &ServerMessage::new(ServerPayload::ContactsChunk {
                    contacts: Vec::<Contact>::new(),
                    done: index == MAX_SNAPSHOT_CHUNKS,
                }),
            )
            .await
            .expect("write contacts chunk");
        }
    });
    let error = timeout(Duration::from_secs(3), client.contacts())
        .await
        .expect("contacts read should be bounded")
        .expect_err("oversized list must not be published");
    assert!(matches!(error, IpcError::SnapshotTooLarge));
    timeout(Duration::from_secs(3), server)
        .await
        .expect("fake server finishes")
        .expect("fake server task succeeds");
}

#[tokio::test]
async fn conversations_reject_more_than_256_chunks() {
    let (mut client, mut server_reader, mut server_writer) = client_pair().await;
    let server = tokio::spawn(async move {
        assert_eq!(
            read_request(&mut server_reader).await.method,
            Method::MessagesConversations {
                account_id: MessagingAccountId::new("account")
            }
        );
        for index in 0..=MAX_SNAPSHOT_CHUNKS {
            write_json_line(
                &mut server_writer,
                &ServerMessage::new(ServerPayload::ConversationsChunk {
                    conversations: vec![],
                    done: index == MAX_SNAPSHOT_CHUNKS,
                }),
            )
            .await
            .expect("write conversations chunk");
        }
    });
    let error = timeout(
        Duration::from_secs(3),
        client.messaging_conversations(MessagingAccountId::new("account")),
    )
    .await
    .expect("conversation read should be bounded")
    .expect_err("oversized list must not be published");
    assert!(matches!(error, IpcError::SnapshotTooLarge));
    timeout(Duration::from_secs(3), server)
        .await
        .expect("fake server finishes")
        .expect("fake server task succeeds");
}

#[tokio::test]
async fn outgoing_operations_reject_more_than_256_chunks() {
    let (mut client, mut server_reader, mut server_writer) = client_pair().await;
    let server = tokio::spawn(async move {
        assert_eq!(
            read_request(&mut server_reader).await.method,
            Method::MessagesOutgoing
        );
        for index in 0..=MAX_SNAPSHOT_CHUNKS {
            write_json_line(
                &mut server_writer,
                &ServerMessage::new(ServerPayload::OutgoingOperations {
                    operations: vec![],
                    done: index == MAX_SNAPSHOT_CHUNKS,
                }),
            )
            .await
            .expect("write outgoing operations chunk");
        }
    });
    let error = timeout(Duration::from_secs(3), client.messaging_outgoing())
        .await
        .expect("outgoing operation read should be bounded")
        .expect_err("oversized list must not be published");
    assert!(matches!(error, IpcError::SnapshotTooLarge));
    timeout(Duration::from_secs(3), server)
        .await
        .expect("fake server finishes")
        .expect("fake server task succeeds");
}

#[tokio::test]
async fn json_errors_keep_position_without_rejected_content() {
    let input = b"{\"private-sentinel\": invalid}\n";
    let mut reader = BufReader::new(input.as_slice());
    let error = read_json_line::<_, ServerMessage>(&mut reader)
        .await
        .expect_err("invalid JSON must be rejected");
    for rendered in [error.to_string(), format!("{error:?}")] {
        assert!(!rendered.contains("private-sentinel"));
        assert!(rendered.contains("line"));
        assert!(rendered.contains("column"));
    }
}

#[test]
fn unexpected_payload_errors_do_not_include_payload_contents() {
    let error = unexpected(ServerPayload::Notifications {
        notifications: vec![notification("private-notification-sentinel")],
    });
    for rendered in [error.to_string(), format!("{error:?}")] {
        assert!(!rendered.contains("private-notification-sentinel"));
        assert!(rendered.contains("message type does not match"));
    }
}

#[tokio::test]
async fn notification_list_chunks_are_assembled_in_order() {
    let (mut client, mut reader, mut writer) = client_pair().await;
    let first = notification("first");
    let mut second = notification("second");
    second.id.local_id = "second".into();
    let expected = vec![first.clone(), second.clone()];
    let server = tokio::spawn(async move {
        assert_eq!(
            read_request(&mut reader).await.method,
            Method::NotificationsList
        );
        for (item, done) in [(first, false), (second, true)] {
            write_json_line(
                &mut writer,
                &ServerMessage::new(ServerPayload::NotificationsChunk {
                    notifications: vec![item],
                    done,
                }),
            )
            .await
            .unwrap();
        }
    });
    assert_eq!(client.notifications().await.unwrap(), expected);
    server.await.unwrap();
}

#[tokio::test]
async fn media_list_chunks_are_assembled_in_order() {
    let (mut client, mut reader, mut writer) = client_pair().await;
    let first = MediaSession {
        id: MediaSessionId::new(DeviceId::new("phone"), "player"),
        application: "App".into(),
        title: Some("First".into()),
        artist: None,
        album: None,
        playback: handover_core::PlaybackState::Paused,
        position_ms: None,
        duration_ms: None,
        volume_percent: None,
        controls: Default::default(),
    };
    let mut second = first.clone();
    second.id.player_id = "second".into();
    let expected = vec![first.clone(), second.clone()];
    let server = tokio::spawn(async move {
        assert_eq!(read_request(&mut reader).await.method, Method::MediaList);
        for (item, done) in [(first, false), (second, true)] {
            write_json_line(
                &mut writer,
                &ServerMessage::new(ServerPayload::MediaChunk {
                    media_sessions: vec![item],
                    done,
                }),
            )
            .await
            .unwrap();
        }
    });
    assert_eq!(client.media_sessions().await.unwrap(), expected);
    server.await.unwrap();
}

#[tokio::test]
async fn a_full_list_cannot_complete_an_unfinished_chunk_sequence() {
    let (mut client, mut reader, mut writer) = client_pair().await;
    let server = tokio::spawn(async move {
        read_request(&mut reader).await;
        write_json_line(
            &mut writer,
            &ServerMessage::new(ServerPayload::NotificationsChunk {
                notifications: vec![notification("private-prefix")],
                done: false,
            }),
        )
        .await
        .unwrap();
        write_json_line(
            &mut writer,
            &ServerMessage::new(ServerPayload::Notifications {
                notifications: vec![notification("private-full-list")],
            }),
        )
        .await
        .unwrap();
    });
    let error = client.notifications().await.unwrap_err();
    assert!(matches!(error, IpcError::UnexpectedResponse(_)));
    assert!(!format!("{error:?}").contains("private"));
    server.await.unwrap();
}
