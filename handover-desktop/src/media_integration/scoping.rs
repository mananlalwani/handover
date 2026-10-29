use super::*;

#[tokio::test]
async fn identical_player_ids_remain_device_scoped_through_removal_and_snapshot() {
    let (_bus, address) = private_bus().await;
    let connection = client(&address).await;
    let bus = Proxy::new(
        &connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await
    .unwrap();
    let mut owners = bus.receive_signal("NameOwnerChanged").await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("daemon.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let (shutdown, receiver) = watch::channel(false);
    let worker = tokio::spawn(run_media(path, address, receiver));

    let first = session();
    let mut second = session();
    second.id.device_id = DeviceId::new("phone-b");
    let mut second_device = device();
    second_device.id = second.id.device_id.clone();
    second_device.name = "Phone B".into();

    let mut daemon = subscribe(&listener, vec![]).await;
    send(
        &mut daemon,
        ServerPayload::DeviceAdded {
            device: second_device.clone(),
        },
    )
    .await;
    send(
        &mut daemon,
        ServerPayload::MediaAdded {
            media_session: first.clone(),
        },
    )
    .await;
    let first_name = "org.mpris.MediaPlayer2.handover.player1";
    owner(&mut owners, first_name, true).await;
    send(
        &mut daemon,
        ServerPayload::MediaAdded {
            media_session: second.clone(),
        },
    )
    .await;
    let second_name = "org.mpris.MediaPlayer2.handover.player2";
    owner(&mut owners, second_name, true).await;

    let second_player = Proxy::new(&connection, second_name, PATH, PLAYER)
        .await
        .unwrap();
    let (result, ()) = tokio::join!(
        second_player.call::<_, _, ()>("Pause", &()),
        command(
            &listener,
            MediaCommand::Pause {
                id: second.id.clone(),
            },
        )
    );
    result.unwrap();

    send(
        &mut daemon,
        ServerPayload::DeviceRemoved {
            device_id: first.id.device_id.clone(),
        },
    )
    .await;
    owner(&mut owners, first_name, false).await;

    let (result, ()) = tokio::join!(
        second_player.call::<_, _, ()>("Next", &()),
        command(
            &listener,
            MediaCommand::Next {
                id: second.id.clone(),
            },
        )
    );
    result.unwrap();

    send(
        &mut daemon,
        ServerPayload::Snapshot {
            devices: vec![second_device],
            notifications: vec![],
            media_sessions: vec![],
            calls: vec![],
            messaging_accounts: vec![],
            conversations: vec![],
            typing_states: vec![],
            read_states: vec![],
        },
    )
    .await;
    owner(&mut owners, second_name, false).await;
    assert!(
        timeout(Duration::from_millis(150), listener.accept())
            .await
            .is_err(),
        "session withdrawal must not issue a phone command"
    );

    shutdown.send(true).unwrap();
    timeout(CALL_TIMEOUT, worker).await.unwrap().unwrap();
}
