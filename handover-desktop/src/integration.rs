use super::*;
use handover_core::{DeviceId, NotificationAction};
use handover_ipc::{Method, Request, ServerMessage, read_json_line, write_json_line};
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::{Child, Command};
use zbus::{interface, object_server::SignalEmitter, zvariant::OwnedValue};

#[derive(Debug)]
enum DesktopEvent {
    Posted {
        id: u32,
        replaces: u32,
        body: String,
        actions: Vec<String>,
    },
    Closed(u32),
}

struct Notifications {
    next_id: AtomicU32,
    events: mpsc::Sender<DesktopEvent>,
}

#[interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    fn get_capabilities(&self) -> Vec<&str> {
        vec!["body", "body-markup", "actions"]
    }

    #[allow(clippy::too_many_arguments)] // The standard D-Bus Notify signature.
    async fn notify(
        &self,
        _app: &str,
        replaces: u32,
        _icon: &str,
        _summary: &str,
        body: &str,
        actions: Vec<String>,
        _hints: HashMap<String, OwnedValue>,
        _timeout: i32,
    ) -> u32 {
        let id = if replaces == 0 {
            self.next_id.fetch_add(1, Ordering::SeqCst)
        } else {
            replaces
        };
        self.events
            .send(DesktopEvent::Posted {
                id,
                replaces,
                body: body.to_owned(),
                actions,
            })
            .await
            .unwrap();
        id
    }

    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let _ = self.events.send(DesktopEvent::Closed(id)).await;
        Self::notification_closed(&emitter, id, 3).await.unwrap();
    }

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;
}

pub(super) struct Bus(Child);
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.0.start_kill();
    }
}

pub(super) async fn private_bus() -> (Bus, String) {
    let mut child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--nopidfile", "--print-address"])
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("dbus-daemon is required for desktop integration tests");
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut address = String::new();
    timeout(CALL_TIMEOUT, output.read_line(&mut address))
        .await
        .unwrap()
        .unwrap();
    (Bus(child), address.trim().to_owned())
}

fn notification(id: &str, text: &str) -> Notification {
    Notification {
        id: NotificationId::new(DeviceId::new("fixture"), id),
        app_name: "Fixture".into(),
        title: "Title".into(),
        body: text.into(),
        icon_path: None,
        clearable: true,
        actions: vec![NotificationAction {
            id: "open".into(),
            label: "Open".into(),
        }],
        reply_supported: true,
    }
}

async fn subscribe(listener: &UnixListener, notifications: Vec<Notification>) -> UnixStream {
    let (mut socket, _) = timeout(CALL_TIMEOUT, listener.accept())
        .await
        .unwrap()
        .unwrap();
    let request: Request = read_json_line(&mut BufReader::new(&mut socket))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        request.method,
        Method::Subscribe {
            shares: false,
            media: false,
            messages: false
        }
    );
    write_json_line(
        &mut socket,
        &ServerMessage::new(ServerPayload::Subscribed {
            devices: vec![],
            notifications,
            media_sessions: vec![],
            calls: vec![],
            messaging_accounts: vec![],
            conversations: vec![],
            typing_states: vec![],
            read_states: vec![],
        }),
    )
    .await
    .unwrap();
    socket
}

async fn send(socket: &mut UnixStream, payload: ServerPayload) {
    write_json_line(socket, &ServerMessage::new(payload))
        .await
        .unwrap();
}

async fn next(events: &mut mpsc::Receiver<DesktopEvent>) -> DesktopEvent {
    timeout(CALL_TIMEOUT, events.recv()).await.unwrap().unwrap()
}

#[tokio::test]
async fn notifications_follow_daemon_state_and_only_explicit_actions_reach_ipc() {
    let (_bus, address) = private_bus().await;
    let (events, mut received) = mpsc::channel(32);
    let server = zbus::connection::Builder::address(address.as_str())
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
    let old = notification("old", "historical");
    let mut socket = subscribe(&listener, vec![old.clone()]).await;
    // A duplicate added event queued during subscribe must not replay history.
    send(
        &mut socket,
        ServerPayload::NotificationAdded { notification: old },
    )
    .await;
    let note = notification("new", "<b>hello & goodbye</b>");
    send(
        &mut socket,
        ServerPayload::NotificationAdded {
            notification: note.clone(),
        },
    )
    .await;
    let id = match next(&mut received).await {
        DesktopEvent::Posted {
            id,
            replaces,
            body,
            actions,
        } => {
            assert_eq!(replaces, 0);
            assert_eq!(body, "&lt;b&gt;hello &amp; goodbye&lt;/b&gt;");
            assert_eq!(
                actions,
                vec!["action:open", "Open", "dismiss", "Dismiss on phone"]
            );
            id
        }
        other => panic!("{other:?}"),
    };
    let emitter = server
        .object_server()
        .interface::<_, Notifications>(PATH)
        .await
        .unwrap();
    Notifications::action_invoked(emitter.signal_emitter(), id, "action:open")
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
            notification_id: note.id.clone(),
            action_id: "open".into()
        }
    );
    write_json_line(
        &mut action,
        &ServerMessage::new(ServerPayload::CommandCompleted {
            notification_id: note.id.clone(),
        }),
    )
    .await
    .unwrap();
    // D-Bus actions must not cancel a partially consumed IPC frame.
    let changed = notification("new", "changed");
    let mut frame = serde_json::to_vec(&ServerMessage::new(ServerPayload::NotificationUpdated {
        notification: changed,
    }))
    .unwrap();
    frame.push(b'\n');
    let split = frame.len() / 2;
    socket.write_all(&frame[..split]).await.unwrap();
    // Explicit phone dismissal is distinct from dismissing the local popup.
    Notifications::action_invoked(emitter.signal_emitter(), id, "dismiss")
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
        Method::NotificationDismiss {
            notification_id: note.id.clone()
        }
    );
    write_json_line(
        &mut action,
        &ServerMessage::new(ServerPayload::CommandCompleted {
            notification_id: note.id.clone(),
        }),
    )
    .await
    .unwrap();
    // A new revision replaces the existing desktop item rather than duplicating it.
    socket.write_all(&frame[split..]).await.unwrap();
    match next(&mut received).await {
        DesktopEvent::Posted { replaces, .. } => assert_eq!(replaces, id),
        other => panic!("{other:?}"),
    }
    Notifications::notification_closed(emitter.signal_emitter(), id, 2)
        .await
        .unwrap();
    // No remote dismissal and no re-alert on an identical snapshot.
    send(
        &mut socket,
        ServerPayload::Snapshot {
            devices: vec![],
            notifications: vec![notification("new", "changed")],
            media_sessions: vec![],
            calls: vec![],
            messaging_accounts: vec![],
            conversations: vec![],
            typing_states: vec![],
            read_states: vec![],
        },
    )
    .await;
    let last = notification("last", "last");
    send(
        &mut socket,
        ServerPayload::NotificationAdded {
            notification: last.clone(),
        },
    )
    .await;
    let last_id = match next(&mut received).await {
        DesktopEvent::Posted { id, .. } => id,
        other => panic!("{other:?}"),
    };
    send(
        &mut socket,
        ServerPayload::NotificationRemoved {
            notification_id: last.id,
        },
    )
    .await;
    assert!(matches!(next(&mut received).await, DesktopEvent::Closed(id) if id == last_id));
    assert!(
        timeout(Duration::from_millis(80), listener.accept())
            .await
            .is_err(),
        "local close must not issue a phone command"
    );
    shutdown.send(true).unwrap();
    timeout(CALL_TIMEOUT, client).await.unwrap().unwrap();
}

mod recovery;
