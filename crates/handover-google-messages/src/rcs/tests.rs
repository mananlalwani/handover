use super::*;
use handover_core::messaging::{ConversationId, MessagingAccountId, Participant};
use std::collections::BTreeSet;

fn conversation(id: &str) -> Conversation {
    Conversation {
        id: ConversationId::new(MessagingAccountId::new("test"), id),
        kind: ConversationKind::Direct,
        transport: TransportKind::Rcs,
        title: None,
        participants: vec![Participant {
            local_id: "peer".into(),
            display_name: None,
            address: Some("+15550000001".into()),
            is_self: false,
        }],
        latest_message_id: None,
        last_activity_at: None,
        unread_count: None,
        cursor: None,
        capabilities: BTreeSet::new(),
    }
}
fn record(mime: &str, body: &[u8]) -> Value {
    let content = ContentBatch {
        contents: vec![Content {
            mime: mime.into(),
            data: body.to_vec(),
        }],
    }
    .encode_to_vec();
    let mut message = vec![Value::Null; 12];
    message[0] = json!("synthetic-inbox");
    message[1] = json!(36);
    message[2] = json!("1000000000");
    message[7] = json!([1, "+15550000001", "RCS"]);
    message[11] = json!(STANDARD.encode(content));
    json!([[], message])
}
fn decode_record(record: Value) -> Result<Option<TypingEvent>, ReceiveError> {
    let mut found = None;
    let mut stream = crate::receive::ReceiveStream::default();
    let bytes = serde_json::to_vec(&json!([[record], [0]])).unwrap();
    stream.feed(&bytes, |event| {
        if let crate::receive::ReceiveEvent::Record(record) = event {
            found = typing(&record)?;
        }
        Ok(())
    })?;
    stream.finish()?;
    Ok(found)
}
#[test]
fn only_composing_content_with_an_rcs_sender_produces_typing() {
    let body = b"<isComposing xmlns='urn:ietf:params:xml:ns:im-iscomposing'><state>active</state><refresh>60</refresh></isComposing>";
    let decoded = decode_record(record(
        "application/im-iscomposing+xml; charset=utf-8",
        body,
    ))
    .unwrap()
    .unwrap();
    assert_eq!(decoded.sender.as_str(), "+15550000001");
    assert_eq!(
        decoded.composing,
        composing::Composing::Active(Duration::from_secs(60))
    );
    assert!(
        decode_record(record("text/plain", b"ordinary message"))
            .unwrap()
            .is_none()
    );
    let mut grouped = record("application/im-iscomposing+xml", body);
    grouped[1][9] = json!([2, "group", "RCS"]);
    assert!(decode_record(grouped).unwrap().is_none());
    let mut foreign = record("application/im-iscomposing+xml", body);
    foreign[1][7][2] = json!("GDitto");
    assert!(decode_record(foreign).unwrap().is_none());
    let mut ambiguous = record("application/im-iscomposing+xml", body);
    ambiguous.as_array_mut().unwrap().push(json!([]));
    assert!(matches!(
        decode_record(ambiguous),
        Err(ReceiveError::InvalidEvents)
    ));
}
#[test]
fn sender_binding_requires_one_attested_direct_rcs_conversation() {
    let known = vec![conversation("one")];
    assert_eq!(bind(&known, "+15550000001"), Some(("one", "peer")));
    assert!(bind(&known, "+15550000002").is_none());
    assert!(bind(&[conversation("one"), conversation("two")], "+15550000001").is_none());
    let mut group = conversation("one");
    group.kind = ConversationKind::Group;
    assert!(bind(&[group], "+15550000001").is_none());
    let mut sms = conversation("one");
    sms.transport = TransportKind::Sms;
    assert!(bind(&[sms], "+15550000001").is_none());
}
#[test]
fn old_future_and_reordered_inbox_items_never_revive_typing() {
    let mut event = TypingEvent {
        inbox: Zeroizing::new("id".into()),
        sender: Zeroizing::new("peer".into()),
        composing: composing::Composing::Active(Duration::from_secs(60)),
        timestamp_micros: 1_000_000_000,
    };
    assert_eq!(
        fresh(&event, Duration::from_secs(1010), None),
        Some(Duration::from_secs(50))
    );
    assert_eq!(fresh(&event, Duration::from_secs(1060), None), None);
    assert_eq!(fresh(&event, Duration::from_secs(990), None), None);
    assert_eq!(
        fresh(
            &event,
            Duration::from_secs(1010),
            Some(event.timestamp_micros)
        ),
        None
    );
    event.composing = composing::Composing::Idle;
    assert!(
        fresh(
            &event,
            Duration::from_secs(1010),
            Some(event.timestamp_micros + 1)
        )
        .is_none()
    );
}
#[test]
fn settings_select_only_a_single_authenticated_rcs_identity_and_google_service() {
    let bytes = SettingsUpdate {
        settings: Some(Settings {
            sims: vec![Sim {
                identity: Some(SimIdentity {
                    address: "+15550000001".into(),
                }),
                endpoint: "instantmessaging-pa-preprod.googleapis.com:443".into(),
            }],
        }),
    }
    .encode_to_vec();
    let config = settings(&bytes).unwrap().unwrap();
    assert_eq!(
        config.endpoint,
        "https://instantmessaging-pa.clients6.google.com"
    );
    assert_eq!(config.identity.as_str(), "+15550000001");
    for url in [
        "https://evil.example",
        "https://instantmessaging-pa.googleapis.com.evil.example",
        "https://user@instantmessaging-pa.googleapis.com",
        "https://instantmessaging-pa.googleapis.com/other",
        "http://instantmessaging-pa.googleapis.com",
        "https://instantmessaging-pa.googleapis.com?token=x",
    ] {
        assert!(endpoint(url).is_err());
    }
    let context = Context::default();
    context.settings(&bytes);
    assert!(context.settings.borrow().is_some());
    context.settings(b"malformed");
    assert!(context.settings.borrow().is_none());
}

async fn request(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        socket.read_exact(&mut byte).await.unwrap();
        header.push(byte[0]);
        assert!(header.len() < 8192);
    }
    let header = String::from_utf8(header).unwrap();
    let length: usize = header
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|value| value.trim().parse().unwrap())
        })
        .unwrap();
    assert!(length < 8192);
    let mut body = vec![0; length];
    socket.read_exact(&mut body).await.unwrap();
    body
}

#[tokio::test]
async fn receive_publishes_before_ack_and_expires_without_an_idle_packet() {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let mut signal = record("application/im-iscomposing+xml", b"<isComposing xmlns='urn:ietf:params:xml:ns:im-iscomposing'><state>active</state><refresh>1</refresh></isComposing>");
    signal[1][2] = json!(now.as_micros().to_string());
    let prefix = format!("[[{},{}", record("text/plain", b"ordinary message"), signal);
    let (ack_sent, mut ack_received) = mpsc::channel(1);
    let (finish, finished) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let body: Value = serde_json::from_slice(&request(&mut stream).await).unwrap();
        assert_eq!(body[0][2], "RCS");
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json+protobuf\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        stream
            .write_all(format!("{:x}\r\n{prefix}\r\n", prefix.len()).as_bytes())
            .await
            .unwrap();
        let (mut ack, _) = listener.accept().await.unwrap();
        let body: Value = serde_json::from_slice(&request(&mut ack).await).unwrap();
        ack_sent.send(body).await.unwrap();
        ack.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json+protobuf\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]").await.unwrap();
        finished.await.unwrap();
    });
    let transport = Transport {
        http: crate::client(false).unwrap(),
        stream: crate::streaming_client_for(false).unwrap(),
        headers: HeaderMap::new(),
    };
    let config = Config {
        identity: Zeroizing::new("self".into()),
        endpoint: url,
    };
    let registration = Registration {
        token: Zeroizing::new(vec![1, 2, 3]),
        lifetime: Duration::from_secs(3600),
    };
    let (_, known) = watch::channel(vec![conversation("thread")]);
    let (events, mut outputs) = mpsc::channel(1);
    let mut active = BTreeMap::new();
    let network = receive(
        &transport,
        &config,
        &registration,
        known,
        &events,
        &mut active,
    );
    tokio::pin!(network);
    let output = tokio::select! { result = &mut network => panic!("receive ended: {result:?}"), output = outputs.recv() => output.unwrap() };
    assert!(
        matches!(output.event, LiveEvent::Typing { conversation, participants } if conversation == "thread" && participants == ["peer"])
    );
    tokio::select! {
        result = &mut network => panic!("receive ended: {result:?}"),
        _ = ack_received.recv() => panic!("acknowledged before publication"),
        _ = tokio::time::sleep(Duration::from_millis(50)) => {}
    }
    output.accepted.send(()).unwrap();
    let ack = tokio::select! { result = &mut network => panic!("receive ended: {result:?}"), ack = ack_received.recv() => ack.unwrap() };
    assert_eq!(ack[0][2], "RCS");
    assert_eq!(ack[1], json!(["synthetic-inbox"]));
    let cleared = tokio::select! {
        result = &mut network => panic!("receive ended: {result:?}"),
        output = outputs.recv() => output.unwrap(),
        _ = tokio::time::sleep(Duration::from_secs(3)) => panic!("indicator did not expire")
    };
    assert!(
        matches!(cleared.event, LiveEvent::Typing { conversation, participants } if conversation == "thread" && participants.is_empty())
    );
    cleared.accepted.send(()).unwrap();
    finish.send(()).unwrap();
    server.await.unwrap();
}
