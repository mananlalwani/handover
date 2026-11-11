use super::*;

use handover_core::{DeviceId, NotificationAction};
use tokio::io::BufReader;
use tokio::net::{UnixStream, unix::OwnedReadHalf};
use tokio::time::{Duration, timeout};

fn notification(id: &str, body: String) -> Notification {
    Notification {
        id: NotificationId::new(DeviceId::new("phone"), id),
        app_name: "Fixture".into(),
        title: format!("Title {id}"),
        body,
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

fn subscribed_chunk(notifications: Vec<Notification>, done: bool) -> ServerMessage {
    ServerMessage::new(ServerPayload::SubscribedChunk {
        devices: vec![],
        notifications,
        media_sessions: vec![],
        calls: vec![],
        messaging_accounts: vec![],
        conversations: vec![],
        typing_states: vec![],
        read_states: vec![],
        done,
    })
}

fn snapshot_chunk(notifications: Vec<Notification>, done: bool) -> ServerMessage {
    ServerMessage::new(ServerPayload::SnapshotChunk {
        devices: vec![],
        notifications,
        media_sessions: vec![],
        calls: vec![],
        messaging_accounts: vec![],
        conversations: vec![],
        typing_states: vec![],
        read_states: vec![],
        done,
    })
}

async fn assert_notification_subscription(reader: &mut BufReader<OwnedReadHalf>) {
    let request: Request = read_json_line(reader)
        .await
        .expect("read request")
        .expect("request exists");
    assert_eq!(
        request.method,
        Method::Subscribe {
            shares: false,
            media: false,
            messages: false,
        }
    );
}

#[tokio::test]
async fn subscription_rejects_more_than_the_snapshot_chunk_limit() {
    let (client, mut server_reader, mut server_writer) = client_pair().await;
    let server = tokio::spawn(async move {
        assert_notification_subscription(&mut server_reader).await;
        for index in 0..=MAX_SNAPSHOT_CHUNKS {
            write_json_line(
                &mut server_writer,
                &subscribed_chunk(vec![], index == MAX_SNAPSHOT_CHUNKS),
            )
            .await
            .expect("write subscription chunk");
        }
    });

    let result = timeout(Duration::from_secs(3), client.subscribe_notifications())
        .await
        .expect("subscription read should be bounded");
    let error = match result {
        Ok(_) => panic!("oversized subscription snapshot must fail"),
        Err(error) => error,
    };
    assert!(matches!(error, IpcError::SnapshotTooLarge));
    timeout(Duration::from_secs(3), server)
        .await
        .expect("fake server finishes")
        .expect("fake server task succeeds");
}

#[tokio::test]
async fn recovery_snapshot_overflow_returns_no_partial_snapshot() {
    let (client, mut server_reader, mut server_writer) = client_pair().await;
    let server = tokio::spawn(async move {
        assert_notification_subscription(&mut server_reader).await;
        write_json_line(
            &mut server_writer,
            &ServerMessage::new(ServerPayload::Subscribed {
                devices: vec![],
                notifications: vec![],
                media_sessions: vec![],
                calls: vec![],
                messaging_accounts: vec![],
                conversations: vec![],
                typing_states: vec![],
                read_states: vec![],
            }),
        )
        .await
        .expect("write initial subscription");
        for index in 0..=MAX_SNAPSHOT_CHUNKS {
            write_json_line(
                &mut server_writer,
                &snapshot_chunk(vec![], index == MAX_SNAPSHOT_CHUNKS),
            )
            .await
            .expect("write recovery snapshot chunk");
        }
    });
    let mut subscription = timeout(Duration::from_secs(3), client.subscribe_notifications())
        .await
        .expect("subscription read should be bounded")
        .expect("initial subscription fits");

    let error = timeout(Duration::from_secs(3), subscription.next_message())
        .await
        .expect("recovery snapshot read should be bounded")
        .expect_err("oversized recovery snapshot must fail as a whole");
    assert!(matches!(error, IpcError::SnapshotTooLarge));
    timeout(Duration::from_secs(3), server)
        .await
        .expect("fake server finishes")
        .expect("fake server task succeeds");
}

#[tokio::test]
async fn recovery_snapshot_byte_limit_applies_across_below_line_limit_chunks() {
    let (client, mut server_reader, mut server_writer) = client_pair().await;
    let server = tokio::spawn(async move {
        assert_notification_subscription(&mut server_reader).await;
        write_json_line(
            &mut server_writer,
            &ServerMessage::new(ServerPayload::Subscribed {
                devices: vec![],
                notifications: vec![],
                media_sessions: vec![],
                calls: vec![],
                messaging_accounts: vec![],
                conversations: vec![],
                typing_states: vec![],
                read_states: vec![],
            }),
        )
        .await
        .expect("write initial subscription");

        for index in 0..32 {
            let message = snapshot_chunk(
                vec![notification(&index.to_string(), "x".repeat(512 * 1024))],
                index == 31,
            );
            let encoded = serde_json::to_vec(&message).expect("chunk serializes");
            assert!(encoded.len() < MAX_LINE_BYTES);
            write_json_line(&mut server_writer, &message)
                .await
                .expect("write recovery snapshot chunk");
        }
    });
    let mut subscription = timeout(Duration::from_secs(3), client.subscribe_notifications())
        .await
        .expect("subscription read should be bounded")
        .expect("initial subscription fits");

    let error = timeout(Duration::from_secs(15), subscription.next_message())
        .await
        .expect("recovery snapshot byte accounting should be bounded")
        .expect_err("aggregate byte limit must reject the snapshot");
    assert!(matches!(error, IpcError::SnapshotTooLarge));
    timeout(Duration::from_secs(3), server)
        .await
        .expect("fake server finishes")
        .expect("fake server task succeeds");
}

#[tokio::test]
async fn valid_subscription_chunks_reconstruct_the_exact_inventory() {
    let first = notification("first", "one".into());
    let second = notification("second", "two".into());
    let expected = vec![first.clone(), second.clone()];
    let (client, mut server_reader, mut server_writer) = client_pair().await;
    let server = tokio::spawn(async move {
        assert_notification_subscription(&mut server_reader).await;
        write_json_line(&mut server_writer, &subscribed_chunk(vec![first], false))
            .await
            .expect("write first subscription chunk");
        write_json_line(&mut server_writer, &subscribed_chunk(vec![second], true))
            .await
            .expect("write final subscription chunk");
    });

    let subscription = timeout(Duration::from_secs(3), client.subscribe_notifications())
        .await
        .expect("subscription read should be bounded")
        .expect("valid chunked subscription should succeed");
    assert_eq!(subscription.notifications, expected);
    timeout(Duration::from_secs(3), server)
        .await
        .expect("fake server finishes")
        .expect("fake server task succeeds");
}

#[tokio::test]
async fn fragmented_replies_preserve_utf8_and_complete_snapshot_boundaries() {
    let (mut client, mut reader, mut writer) = client_pair().await;
    // Force reads to split even within the multibyte text in each record.
    client.reader = BufReader::with_capacity(1, client.reader.into_inner());
    let first = notification("first", "phone 📱".into());
    let second = notification("second", "reply ✓".into());
    let expected = vec![first.clone(), second.clone()];
    let server = tokio::spawn(async move {
        assert_notification_subscription(&mut reader).await;
        let mut bytes = Vec::new();
        for message in [
            subscribed_chunk(vec![first.clone()], false),
            subscribed_chunk(vec![second.clone()], true),
            snapshot_chunk(vec![second], false),
            snapshot_chunk(vec![first], true),
        ] {
            bytes.extend(serde_json::to_vec(&message).unwrap());
            bytes.push(b'\n');
        }
        writer.write_all(&bytes).await.unwrap();
    });
    let mut subscription = timeout(Duration::from_secs(3), client.subscribe_notifications())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(subscription.notifications, expected);
    let message = timeout(Duration::from_secs(3), subscription.next_message())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(message.payload, ServerPayload::Snapshot { notifications, .. }
        if notifications == expected.into_iter().rev().collect::<Vec<_>>())
    );
    server.await.unwrap();
}

#[tokio::test]
async fn interrupted_subscription_and_recovery_never_publish_partial_state() {
    for recovery in [false, true] {
        for truncated_record in [false, true] {
            let (client, mut reader, mut writer) = client_pair().await;
            let server = tokio::spawn(async move {
                assert_notification_subscription(&mut reader).await;
                if recovery {
                    write_json_line(&mut writer, &subscribed_chunk(vec![], true))
                        .await
                        .unwrap();
                }
                let chunk = if recovery {
                    snapshot_chunk
                } else {
                    subscribed_chunk
                };
                write_json_line(
                    &mut writer,
                    &chunk(
                        vec![notification("partial", "private-prefix".into())],
                        false,
                    ),
                )
                .await
                .unwrap();
                if truncated_record {
                    writer
                        .write_all(b"{\"protocol\":1,\"type\":")
                        .await
                        .unwrap();
                }
            });
            let error = timeout(Duration::from_secs(3), async {
                let result = client.subscribe_notifications().await;
                if recovery {
                    result.unwrap().next_message().await.err().unwrap()
                } else {
                    result.err().unwrap()
                }
            })
            .await
            .expect("interrupted snapshot must terminate");
            if truncated_record {
                assert!(matches!(error, IpcError::Json(_)));
            } else {
                assert!(matches!(error, IpcError::ConnectionClosed));
            }
            assert!(!format!("{error:?}").contains("private-prefix"));
            server.await.unwrap();
        }
    }
}

#[tokio::test]
async fn recovery_rejects_interleaved_events_and_mixed_chunk_kinds() {
    for intervening in [
        subscribed_chunk(vec![], true),
        ServerMessage::new(ServerPayload::Devices { devices: vec![] }),
    ] {
        let (client, mut reader, mut writer) = client_pair().await;
        let server = tokio::spawn(async move {
            assert_notification_subscription(&mut reader).await;
            for message in [
                subscribed_chunk(vec![], true),
                snapshot_chunk(
                    vec![notification("partial", "private-prefix".into())],
                    false,
                ),
                intervening,
            ] {
                write_json_line(&mut writer, &message).await.unwrap();
            }
        });
        let mut subscription = client.subscribe_notifications().await.unwrap();
        let error = timeout(Duration::from_secs(3), subscription.next_message())
            .await
            .unwrap()
            .unwrap_err();
        assert!(matches!(error, IpcError::UnexpectedResponse(_)));
        assert!(!format!("{error:?}").contains("private-prefix"));
        server.await.unwrap();
    }
}
