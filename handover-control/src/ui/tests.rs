use super::*;
use handover_ipc::{Method, Request as IpcRequest, ServerMessage, read_json_line, write_json_line};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::BufReader,
    net::UnixListener,
    sync::{broadcast, watch},
};

fn device(online: bool) -> Device {
    Device {
        id: DeviceId::new(if online { "phone-a" } else { "phone-b" }),
        name: if online {
            "Pixel test"
        } else {
            "Offline phone"
        }
        .into(),
        connected: online,
        paired: true,
        battery: Some(BatteryState::new(81, true).unwrap()),
        connectivity: None,
        capabilities: BTreeSet::from([
            Capability::Battery,
            Capability::FileTransfer,
            Capability::Notifications,
            Capability::Media,
        ]),
    }
}
fn note(online: bool) -> Notification {
    Notification {
        id: NotificationId::new(device(online).id, "fixture-note"),
        app_name: "Battery monitor".into(),
        title: if online {
            "Battery at 81%"
        } else {
            "Offline notification"
        }
        .into(),
        body: "Fictional notification, never sent to a phone.".into(),
        icon_path: None,
        clearable: true,
        actions: vec![],
        reply_supported: false,
    }
}
fn baseline() -> ServerPayload {
    ServerPayload::Subscribed {
        devices: vec![device(true), device(false)],
        notifications: vec![note(true), note(false)],
        media_sessions: vec![],
        calls: vec![],
        messaging_accounts: vec![],
        conversations: vec![],
        typing_states: vec![],
        read_states: vec![],
    }
}
fn spin(mut condition: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(8);
    while !condition() {
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        assert!(Instant::now() < end, "GUI condition timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
    while glib::MainContext::default().pending() {
        glib::MainContext::default().iteration(false);
    }
}
fn descendants(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let root = root.as_ref();
    let mut items = vec![root.clone()];
    let mut child = root.first_child();
    while let Some(widget) = child {
        items.extend(descendants(&widget));
        child = widget.next_sibling();
    }
    items
}
fn button(root: &impl IsA<gtk::Widget>, label: &str) -> gtk::Button {
    descendants(root)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Button>().ok())
        .find(|b| b.label().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("Button not found: {label}"))
}
fn has_text(root: &impl IsA<gtk::Widget>, value: &str) -> bool {
    descendants(root)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Label>().ok())
        .any(|l| l.text().contains(value))
}

#[test]
#[ignore = "requires the private display and IPC fixture runner"]
fn native_control_center_uses_daemon_state_and_never_replays_commands() {
    gtk::init().unwrap();
    if std::env::var_os("HANDOVER_CONTROL_TEST_DARK").is_some() {
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    }
    let root = std::path::PathBuf::from(std::env::var_os("HANDOVER_CONTROL_TEST_ROOT").unwrap());
    let path = handover_ipc::socket_path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let (events, _) = broadcast::channel::<ServerPayload>(32);
    let worker_events = events.clone();
    let (mutations, received) = mpsc::channel::<Method>();
    let offline = Arc::new(AtomicBool::new(false));
    let worker_offline = offline.clone();
    let (reset, watcher) = watch::channel(0u64);
    let (ready, started) = mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener=UnixListener::bind(path).unwrap();ready.send(()).unwrap();
            loop {
                let (socket,_)=listener.accept().await.unwrap();if worker_offline.load(Ordering::SeqCst){drop(socket);continue;}
                let events=worker_events.clone();let mutations=mutations.clone();let mut reset=watcher.clone();
                tokio::spawn(async move {
                    let mut socket=BufReader::new(socket);
                    while let Ok(Some(request))=read_json_line::<_,IpcRequest>(&mut socket).await {
                        let payload=match request.method {
                            Method::Subscribe {shares,media,messages}=>{
                                assert!(shares&&media&&!messages);
                                let mut updates=events.subscribe();reset.borrow_and_update();
                                write_json_line(socket.get_mut(),&ServerMessage::new(baseline())).await.unwrap();
                                loop {tokio::select! {
                                    update=updates.recv()=>{let Ok(update)=update else {break;};if write_json_line(socket.get_mut(),&ServerMessage::new(update)).await.is_err(){break;}},
                                    _=reset.changed()=>break,
                                }}return;
                            },
                            Method::NativePeers=>ServerPayload::NativePeers {peers:vec![]},
                            Method::NativePending=>ServerPayload::NativePending {pending:vec![handover_ipc::NativePendingPeer {id:"pending-test".into(),name:"New phone".into(),code:"12345678".into()}]},
                            Method::ContactsList=>ServerPayload::Contacts {contacts:vec![Contact {device_id:device(true).id,local_id:"alex".into(),display_name:"Alex Example".into(),phones:vec!["+1 312 555 0100".into()],emails:vec![],photo:None}]},
                            Method::TransfersList=>ServerPayload::TransferActivity {activities:vec![TransferActivity {sequence:1,observed_at_ms:1_800_000_000_000,event:TransferActivityEvent::Result {result:ShareResult {device_id:device(true).id,transfer_id:"fixture-transfer".into(),status:ShareStatus::Completed,reason:None}}}]},
                            method=>{
                                mutations.send(method.clone()).unwrap();
                                match method {
                                    Method::NotificationDismiss {notification_id}=>{let _=events.send(ServerPayload::NotificationRemoved {notification_id:notification_id.clone()});ServerPayload::CommandCompleted {notification_id}},
                                    Method::ShareUrl {device_id,..}=>ServerPayload::ShareAccepted {device_id,transfer_id:Some("fixture-share".into())},
                                    Method::ClipboardSend {..}=>return, // Simulate a lost acknowledgement after consuming the mutation.
                                    Method::NativePair {..}|Method::ContactsSync {..}=>ServerPayload::NativeAccepted,
                                    _=>ServerPayload::NativeAccepted,
                                }
                            }
                        };
                        if write_json_line(socket.get_mut(),&ServerMessage::new(payload)).await.is_err(){break;}
                    }
                });
            }
        });
    });
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    let app = adw::Application::builder()
        .application_id("org.handover.ControlTest")
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let ui = Window::new(&app);
    ui.window.set_default_size(
        std::env::var("HANDOVER_CONTROL_TEST_WIDTH")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1060),
        740,
    );
    ui.window.present();
    spin(|| {
        ui.model.borrow().connected
            && ui.model.borrow().contacts.len() == 1
            && ui.model.borrow().pairing.len() == 1
            && ui.pending.borrow().is_empty()
    });
    assert!(has_text(&ui.body, "81% battery"));
    assert!(received.try_recv().is_err());
    // Actions are gated by actual device connection, not the presence of an old notification.
    ui.page.set(Page::Notifications);
    ui.render();
    let buttons = descendants(&ui.body)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Button>().ok())
        .filter(|b| b.label().as_deref() == Some("Dismiss on phone"))
        .collect::<Vec<_>>();
    assert_eq!(buttons.len(), 2);
    assert_eq!(buttons.iter().filter(|b| b.is_sensitive()).count(), 1);
    buttons
        .into_iter()
        .find(|b| b.is_sensitive())
        .unwrap()
        .emit_clicked();
    spin(|| ui.model.borrow().notifications.len() == 1 && ui.pending.borrow().is_empty());
    assert!(
        matches!(received.try_recv().unwrap(),Method::NotificationDismiss {notification_id} if notification_id==note(true).id)
    );
    // A pairing code is visible, but requires an explicit comparison before approval.
    ui.page.set(Page::Connection);
    ui.render();
    assert!(has_text(&ui.body, "12345678"));
    assert!(!button(&ui.body, "Pair device").is_sensitive());
    descendants(&ui.body)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::CheckButton>().ok())
        .unwrap()
        .set_active(true);
    button(&ui.body, "Pair device").emit_clicked();
    spin(|| ui.pending.borrow().is_empty());
    assert_eq!(
        received.try_recv().unwrap(),
        Method::NativePair {
            id: "pending-test".into(),
            code: "12345678".into()
        }
    );
    // Only this explicit dialog submission sends a URL.
    *ui.selected.borrow_mut() = Some(device(true).id);
    ui.page.set(Page::Sharing);
    ui.render();
    button(&ui.body, "Send link").emit_clicked();
    let dialog = app
        .windows()
        .into_iter()
        .find_map(|w| w.downcast::<adw::MessageDialog>().ok())
        .unwrap();
    descendants(&dialog)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Entry>().ok())
        .unwrap()
        .set_text("https://example.test/shared");
    dialog.response("send");
    spin(|| ui.pending.borrow().is_empty());
    assert_eq!(
        received.try_recv().unwrap(),
        Method::ShareUrl {
            device_id: device(true).id,
            url: "https://example.test/shared".into()
        }
    );
    assert!(ui.model.borrow().notice.contains("accepted"));
    assert!(has_text(&ui.body, "Phone confirmed receipt"));
    // Review clipboard contents explicitly, then lose the reply after the fixture consumes the command.
    ui.window.clipboard().set_text("Fictional clipboard text");
    button(&ui.body, "Send clipboard text").emit_clicked();
    let dialog = app
        .windows()
        .into_iter()
        .find_map(|w| w.downcast::<adw::MessageDialog>().ok())
        .unwrap();
    let entry = descendants(&dialog)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Entry>().ok())
        .unwrap();
    spin(|| entry.text() == "Fictional clipboard text");
    assert!(
        received.try_recv().is_err(),
        "opening the clipboard dialog must not send"
    );
    dialog.response("send");
    spin(|| ui.pending.borrow().is_empty());
    assert!(
        matches!(received.try_recv().unwrap(),Method::ClipboardSend {text,..} if text=="Fictional clipboard text")
    );
    assert!(ui.model.borrow().notice.contains("outcome is unknown"));
    // Contact search, copying, and returned snapshots are views of daemon-owned data.
    ui.page.set(Page::Contacts);
    ui.search.set_text("Alex");
    ui.render();
    assert!(has_text(&ui.body, "Alex Example"));
    button(&ui.body, "Copy").emit_clicked();
    let text = glib::MainContext::default()
        .block_on(ui.window.clipboard().read_text_future())
        .unwrap()
        .unwrap();
    assert_eq!(text, "+1 312 555 0100");
    // A replacement snapshot replaces the whole inventory, including removed notifications.
    let snapshot = ServerPayload::Snapshot {
        devices: vec![device(true)],
        notifications: vec![],
        media_sessions: vec![],
        calls: vec![],
        messaging_accounts: vec![],
        conversations: vec![],
        typing_states: vec![],
        read_states: vec![],
    };
    events.send(snapshot).unwrap();
    spin(|| ui.model.borrow().notifications.is_empty());
    offline.store(true, Ordering::SeqCst);
    reset.send(1).unwrap();
    spin(|| !ui.model.borrow().connected);
    ui.page.set(Page::Sharing);
    ui.render();
    assert!(!button(&ui.body, "Send file").is_sensitive());
    assert!(!button(&ui.body, "Send link").is_sensitive());
    offline.store(false, Ordering::SeqCst);
    spin(|| ui.model.borrow().connected && ui.pending.borrow().is_empty());
    assert!(
        received.try_recv().is_err(),
        "reconnect must not replay a mutation"
    );
    // A replacement client reconstructs contacts and transfer activity from the daemon.
    let replacement = Window::new(&app);
    spin(|| {
        replacement.model.borrow().transfers.len() == 1
            && replacement.model.borrow().contacts.len() == 1
    });
    assert!(received.try_recv().is_err());
    replacement.window.close();
    ui.page.set(Page::Overview);
    ui.search.set_text("");
    ui.render();
    if let Some(path) = std::env::var_os("HANDOVER_CONTROL_TEST_SCREENSHOT") {
        let until = Instant::now() + Duration::from_millis(250);
        while Instant::now() < until {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let output = std::process::Command::new("import")
            .args(["-window", "root"])
            .arg(path)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    assert!(root.is_dir());
    ui.window.close();
}
