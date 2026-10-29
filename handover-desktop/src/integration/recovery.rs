use super::*;

#[tokio::test]
async fn daemon_disconnect_closes_popups_and_reconnect_baseline_is_silent() {
    let (_bus, address) = private_bus().await;
    let (events, mut received) = mpsc::channel(32);
    let _server = zbus::connection::Builder::address(address.as_str())
        .unwrap()
        .name(SERVICE)
        .unwrap()
        .serve_at(
            PATH,
            Notifications {
                next_id: AtomicU32::new(1),
                events,
            },
        )
        .unwrap()
        .build()
        .await
        .unwrap();
    let connection = zbus::connection::Builder::address(address.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("daemon.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let (shutdown, receiver) = watch::channel(false);
    let client = tokio::spawn(run(path, connection, receiver));

    let current = notification("current", "current before disconnect");
    let mut daemon = subscribe(&listener, vec![]).await;
    send(
        &mut daemon,
        ServerPayload::NotificationAdded {
            notification: current.clone(),
        },
    )
    .await;
    let desktop_id = match next(&mut received).await {
        DesktopEvent::Posted { id, body, .. } => {
            assert_eq!(body, current.body);
            id
        }
        other => panic!("expected initial popup, received {other:?}"),
    };

    drop(daemon);
    assert!(matches!(
        next(&mut received).await,
        DesktopEvent::Closed(id) if id == desktop_id
    ));

    let mut recovered = timeout(
        Duration::from_secs(8),
        subscribe(&listener, vec![current.clone()]),
    )
    .await
    .expect("client should resubscribe after daemon disconnect");
    let later = notification("later", "live after reconnect");
    send(
        &mut recovered,
        ServerPayload::NotificationAdded {
            notification: later.clone(),
        },
    )
    .await;
    match next(&mut received).await {
        DesktopEvent::Posted { body, replaces, .. } => {
            assert_eq!(body, later.body, "snapshot baseline must stay silent");
            assert_eq!(replaces, 0);
        }
        other => panic!("expected live popup after reconnect, received {other:?}"),
    }

    shutdown.send(true).unwrap();
    timeout(CALL_TIMEOUT, client).await.unwrap().unwrap();
}

#[tokio::test]
async fn notification_service_replacement_does_not_reuse_old_desktop_ids() {
    let (_bus, address) = private_bus().await;
    let (first_events, mut received) = mpsc::channel(32);
    let first_owner = zbus::connection::Builder::address(address.as_str())
        .unwrap()
        .name(SERVICE)
        .unwrap()
        .serve_at(
            PATH,
            Notifications {
                next_id: AtomicU32::new(1),
                events: first_events,
            },
        )
        .unwrap()
        .build()
        .await
        .unwrap();
    let connection = zbus::connection::Builder::address(address.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("daemon.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let (shutdown, receiver) = watch::channel(false);
    let client = tokio::spawn(run(path, connection, receiver));

    let old = notification("old", "old owner popup");
    let mut daemon = subscribe(&listener, vec![]).await;
    send(
        &mut daemon,
        ServerPayload::NotificationAdded {
            notification: old.clone(),
        },
    )
    .await;
    let old_id = match next(&mut received).await {
        DesktopEvent::Posted { id, .. } => id,
        other => panic!("expected old owner popup, received {other:?}"),
    };

    first_owner.close().await.unwrap();
    let (second_events, mut second_received) = mpsc::channel(32);
    let second_owner = zbus::connection::Builder::address(address.as_str())
        .unwrap()
        .name(SERVICE)
        .unwrap()
        .serve_at(
            PATH,
            Notifications {
                next_id: AtomicU32::new(old_id),
                events: second_events,
            },
        )
        .unwrap()
        .build()
        .await
        .unwrap();

    let mut recovered = timeout(
        Duration::from_secs(8),
        subscribe(&listener, vec![old.clone()]),
    )
    .await
    .expect("client should reconnect to replacement notification service");
    let emitter = second_owner
        .object_server()
        .interface::<_, Notifications>(PATH)
        .await
        .unwrap();

    // A replacement service can reuse the previous server's numeric ID. Its
    // stale action signal must not map to the old daemon notification.
    Notifications::action_invoked(emitter.signal_emitter(), old_id, "action:open")
        .await
        .unwrap();
    assert!(
        timeout(Duration::from_millis(150), listener.accept())
            .await
            .is_err()
    );

    let fresh = notification("fresh", "fresh owner popup");
    send(
        &mut recovered,
        ServerPayload::NotificationAdded {
            notification: fresh.clone(),
        },
    )
    .await;
    let fresh_id = match next(&mut second_received).await {
        DesktopEvent::Posted { id, .. } => id,
        other => panic!("expected popup from replacement service, received {other:?}"),
    };
    assert_eq!(fresh_id, old_id, "fixture must exercise numeric ID reuse");
    Notifications::action_invoked(emitter.signal_emitter(), fresh_id, "action:open")
        .await
        .unwrap();

    let (mut action, _) = timeout(CALL_TIMEOUT, listener.accept())
        .await
        .unwrap()
        .unwrap();
    let request: Request = read_json_line(&mut BufReader::new(&mut action))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        request.method,
        Method::NotificationInvokeAction {
            notification_id: fresh.id.clone(),
            action_id: "open".into(),
        }
    );
    write_json_line(
        &mut action,
        &ServerMessage::new(ServerPayload::CommandCompleted {
            notification_id: fresh.id,
        }),
    )
    .await
    .unwrap();

    shutdown.send(true).unwrap();
    timeout(CALL_TIMEOUT, client).await.unwrap().unwrap();
}
