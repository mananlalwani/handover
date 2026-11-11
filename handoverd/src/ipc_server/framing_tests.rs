use super::*;
use handover_core::ShareProgress;
use std::os::fd::AsRawFd;
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn outgoing_event_does_not_discard_a_partial_client_request() {
    fragmented_request(true).await;
}

#[tokio::test]
async fn fragmented_client_request_without_an_intervening_event() {
    fragmented_request(false).await;
}

async fn fragmented_request(intervene: bool) {
    let (client, server) = UnixStream::pair().unwrap();
    let descriptor = server.as_raw_fd();
    let state = Arc::new(RwLock::new(StateStore::default()));
    let (events, _) = broadcast::channel(EVENT_CAPACITY);
    let handler = tokio::spawn(handle_client(server, state, events.clone(), None));
    let (reader, mut writer) = client.into_split();
    let mut reader = BufReader::new(reader);
    write_json_line(
        &mut writer,
        &Request::new(Method::Subscribe {
            shares: true,
            media: false,
            messages: false,
        }),
    )
    .await
    .unwrap();
    let subscribed: ServerMessage = read_json_line(&mut reader).await.unwrap().unwrap();
    assert!(matches!(
        subscribed.payload,
        ServerPayload::Subscribed { .. }
    ));

    // Let the daemon consume the prefix and suspend waiting for its newline.
    writer
        .write_all(b"{\"protocol\":1,\"method\":\"hel")
        .await
        .unwrap();
    timeout(Duration::from_secs(1), async {
        loop {
            let mut unread: libc::c_int = 0;
            // SAFETY: the live handler owns this descriptor throughout the
            // request; FIONREAD writes one c_int into the valid local pointer.
            let result = unsafe { libc::ioctl(descriptor, libc::FIONREAD, &mut unread) };
            assert_eq!(result, 0);
            if unread == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("daemon must consume the request prefix");
    if intervene {
        events
            .send(StateEvent::ShareProgress(ShareProgress {
                device_id: DeviceId::new("fixture"),
                transfer_id: "transfer".into(),
                bytes_sent: 1,
                total_bytes: 2,
            }))
            .unwrap();
        let event: ServerMessage = timeout(Duration::from_secs(1), read_json_line(&mut reader))
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(event.payload, ServerPayload::ShareProgress { .. }));
    }
    writer.write_all(b"lo\"}\n").await.unwrap();
    let reply: ServerMessage = timeout(Duration::from_secs(1), read_json_line(&mut reader))
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(reply.payload, ServerPayload::Hello { .. }),
        "fragmented hello lost its prefix: {:?}",
        reply.payload
    );
    handler.abort();
}

#[tokio::test]
async fn lag_recovery_drops_stale_events_and_does_not_block_another_client() {
    let state = Arc::new(RwLock::new(StateStore::default()));
    let mut device = handover_core::Device {
        id: DeviceId::new("fixture"),
        name: "Fixture".into(),
        connected: true,
        paired: true,
        battery: None,
        connectivity: None,
        capabilities: Default::default(),
    };
    state
        .write()
        .unwrap()
        .apply(StateEvent::Device(DeviceEvent::Added(device.clone())));
    let (events, _) = broadcast::channel(EVENT_CAPACITY);
    let (client, server) = UnixStream::pair().unwrap();
    let handler = tokio::spawn(handle_client(server, state.clone(), events.clone(), None));
    let (reader, mut writer) = client.into_split();
    let mut reader = BufReader::new(reader);
    write_json_line(
        &mut writer,
        &Request::new(Method::Subscribe {
            shares: false,
            media: false,
            messages: false,
        }),
    )
    .await
    .unwrap();
    let subscribed: ServerMessage = read_json_line(&mut reader).await.unwrap().unwrap();
    assert!(matches!(
        subscribed.payload,
        ServerPayload::Subscribed { .. }
    ));

    // This test uses the current-thread runtime. With no await in this burst,
    // the real subscriber must lag beyond the broadcast channel's capacity.
    for percentage in 0..100 {
        device.battery = Some(handover_core::BatteryState::new(percentage, false).unwrap());
        let event = StateEvent::Device(DeviceEvent::Updated(device.clone()));
        state.write().unwrap().apply(event.clone());
        events.send(event).unwrap();
    }
    let (other_client, other_server) = UnixStream::pair().unwrap();
    let other_handler = tokio::spawn(handle_client(
        other_server,
        state.clone(),
        events.clone(),
        None,
    ));
    let (other_reader, mut other_writer) = other_client.into_split();
    let mut other_reader = BufReader::new(other_reader);
    write_json_line(&mut other_writer, &Request::new(Method::DevicesList))
        .await
        .unwrap();
    let response: ServerMessage =
        timeout(Duration::from_secs(3), read_json_line(&mut other_reader))
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    assert!(
        matches!(response.payload, ServerPayload::Devices { devices } if devices == vec![device.clone()])
    );

    let recovered: ServerMessage = timeout(Duration::from_secs(3), read_json_line(&mut reader))
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(recovered.payload, ServerPayload::Snapshot { devices, .. } if devices == vec![device.clone()])
    );
    device.battery = Some(handover_core::BatteryState::new(100, false).unwrap());
    let event = StateEvent::Device(DeviceEvent::Updated(device.clone()));
    state.write().unwrap().apply(event.clone());
    events.send(event).unwrap();
    let next: ServerMessage = timeout(Duration::from_secs(3), read_json_line(&mut reader))
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        next.payload,
        ServerPayload::DeviceUpdated { device },
        "recovery must not replay older events over the replacement snapshot"
    );
    handler.abort();
    other_handler.abort();
}
