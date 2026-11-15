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

const PHOTO: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR4nGP4z8AARAwQCgAf7gP9i18U1AAAAABJRU5ErkJggg==";
fn audio_fixture() -> AudioControlsSnapshot {
    AudioControlsSnapshot {
        settings: AudioRoutingSettings::default(),
        applications: vec![
            AudioApplication {
                id: "browser".into(),
                name: "Browser".into(),
                available: true,
            },
            AudioApplication {
                id: "music".into(),
                name: "Music application".into(),
                available: true,
            },
        ],
        players: vec![AudioDesktopPlayer {
            id: "player-1".into(),
            name: "Desktop music".into(),
        }],
        revision: 1,
        observed_at_ms: 1_800_000_000_000,
        applications_available: true,
        players_available: true,
        settings_available: true,
    }
}
fn check(root: &impl IsA<gtk::Widget>, label: &str) -> gtk::CheckButton {
    descendants(root)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::CheckButton>().ok())
        .find(|b| b.label().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("Check button missing: {label}"))
}
fn expand(root: &impl IsA<gtk::Widget>, label: &str) {
    descendants(root)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Expander>().ok())
        .find(|w| w.label().as_deref() == Some(label))
        .unwrap()
        .set_expanded(true);
}

fn device(online: bool) -> Device {
    Device {
        id: DeviceId::new(if online {
            "native:phone-a"
        } else {
            "native:phone-b"
        }),
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
            Capability::FileDownload,
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
        actions: if online {
            vec![NotificationAction {
                id: "fixture-action".into(),
                label: "Next track".into(),
            }]
        } else {
            vec![]
        },
        reply_supported: online,
    }
}
fn baseline() -> ServerPayload {
    ServerPayload::Subscribed {
        devices: vec![device(true), device(false)],
        notifications: vec![note(true), note(false)],
        media_sessions: vec![MediaSession {
            id: MediaSessionId::new(device(true).id, "music"),
            application: "Music".into(),
            title: Some("Evening walk".into()),
            artist: Some("Demo artist".into()),
            album: None,
            playback: PlaybackState::Playing,
            position_ms: None,
            duration_ms: None,
            volume_percent: None,
            controls: BTreeSet::from([
                MediaControl::Previous,
                MediaControl::Play,
                MediaControl::Pause,
                MediaControl::PlayPause,
                MediaControl::Next,
            ]),
        }],
        calls: vec![CallState {
            device_id: device(true).id,
            phase: CallPhase::Ringing,
            controls: BTreeSet::from([CallAction::Answer, CallAction::Decline]),
            generation: 7,
        }],
        messaging_accounts: vec![],
        conversations: vec![],
        typing_states: vec![],
        read_states: vec![],
    }
}
#[track_caller]
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
        .find(|b| b.label().as_deref() == Some(label) || b.tooltip_text().as_deref() == Some(label))
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
    if std::env::var_os("HANDOVER_CONTROL_TEST_BLACK_ACCENT").is_some() {
        let provider = gtk::CssProvider::new();
        provider.load_from_data("@define-color accent_color #000000;");
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().unwrap(),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );
    }
    let root = std::path::PathBuf::from(std::env::var_os("HANDOVER_CONTROL_TEST_ROOT").unwrap());
    let path = handover_ipc::socket_path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let (events, _) = broadcast::channel::<ServerPayload>(32);
    let worker_events = events.clone();
    let history = Arc::new(std::sync::Mutex::new(vec![
        handover_ipc::ClipboardHistoryEntry {
            id: 1,
            text: "Fictional clipboard entry".into(),
            pinned: false,
            created_at_ms: 1_800_000_000_000,
        },
        handover_ipc::ClipboardHistoryEntry {
            id: 2,
            text: "Pinned clipboard entry".into(),
            pinned: true,
            created_at_ms: 1_800_000_000_001,
        },
    ]));
    let worker_history = history.clone();
    let transfer_log = Arc::new(std::sync::Mutex::new(vec![TransferActivity {
        sequence: 1,
        observed_at_ms: 1_800_000_000_000,
        event: TransferActivityEvent::Result {
            result: ShareResult {
                device_id: device(true).id,
                transfer_id: "fixture-transfer".into(),
                status: ShareStatus::Completed,
                reason: None,
            },
        },
    }]));
    let worker_transfer_log = transfer_log.clone();
    let download_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let worker_download_counter = download_counter.clone();
    let mirror = Arc::new(AtomicBool::new(false));
    let worker_mirror = mirror.clone();
    let audio = Arc::new(std::sync::Mutex::new(audio_fixture()));
    let audio_stream = Arc::new(std::sync::Mutex::new(AudioStreamStatus::idle(
        device(true).id,
    )));
    let reject_audio = Arc::new(AtomicBool::new(false));
    let slow_audio = Arc::new(AtomicBool::new(false));
    let audio_gate = Arc::new(tokio::sync::Notify::new());
    let read_audio_available = Arc::new(AtomicBool::new(true));
    let worker_read_audio_available = read_audio_available.clone();
    let worker_audio = audio.clone();
    let worker_stream = audio_stream.clone();
    let worker_reject = reject_audio.clone();
    let worker_slow = slow_audio.clone();
    let worker_audio_gate = audio_gate.clone();
    let (mutations, received) = mpsc::channel::<Method>();
    let command_gate = Arc::new(tokio::sync::Notify::new());
    let worker_gate = command_gate.clone();
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
                let read_audio_available=worker_read_audio_available.clone();let audio=worker_audio.clone();let audio_stream=worker_stream.clone();let reject=worker_reject.clone();let slow=worker_slow.clone();let audio_gate=worker_audio_gate.clone();
                let mirror=worker_mirror.clone();let history=worker_history.clone();let transfer_log=worker_transfer_log.clone();let download_counter=worker_download_counter.clone();let events=worker_events.clone();let mutations=mutations.clone();let mut reset=watcher.clone();let gate=worker_gate.clone();
                tokio::spawn(async move {
                    let mut socket=BufReader::new(socket);
                    while let Ok(Some(request))=read_json_line::<_,IpcRequest>(&mut socket).await {
                        let payload=match request.method {
                            Method::Subscribe {shares,media,messages}=>{
                                assert!(shares&&media&&!messages);
                                let mut updates=events.subscribe();reset.borrow_and_update();
                                write_json_line(socket.get_mut(),&ServerMessage::new(baseline())).await.unwrap();
                                let request=read_json_line::<_,IpcRequest>(&mut socket).await.unwrap().unwrap();assert!(matches!(request.method,Method::AudioSubscribe));
                                let streams=vec![audio_stream.lock().unwrap().clone()];write_json_line(socket.get_mut(),&ServerMessage::new(ServerPayload::AudioStreams {streams})).await.unwrap();
                                loop {tokio::select! {
                                    update=updates.recv()=>{let Ok(update)=update else {break;};if write_json_line(socket.get_mut(),&ServerMessage::new(update)).await.is_err(){break;}},
                                    _=reset.changed()=>break,
                                }}return;
                            },
                            Method::AudioControls if !read_audio_available.load(Ordering::SeqCst)=>ServerPayload::Error {code:handover_ipc::ErrorCode::BackendUnavailable,message:"fixture unavailable".into(),conversation_id:None},
                            Method::AudioControls=>{let snapshot=audio.lock().unwrap().clone();if slow.swap(false,Ordering::SeqCst){audio_gate.notified().await;}ServerPayload::AudioControls {snapshot}},
                            Method::AudioList=>ServerPayload::AudioStreams {streams:vec![audio_stream.lock().unwrap().clone()]},
                            Method::CustomCommands=>ServerPayload::CustomCommands {commands:vec![CustomCommandEntry {name:"fixture-command".into(),argv:vec!["/bin/true".into()]}]},
                            Method::CallsAudio=>ServerPayload::CallAudio {status:CallAudioStatus {observed:true,gateway_ready:true,duplex_running:false}},
                            Method::ClipboardMirrorStatus=>ServerPayload::ClipboardMirror {enabled:mirror.load(Ordering::SeqCst)},
                            Method::ClipboardHistoryList=>ServerPayload::ClipboardHistory {entries:history.lock().unwrap().clone()},
                            Method::NativePeers=>ServerPayload::NativePeers {peers:vec![]},
                            Method::NativePending=>ServerPayload::NativePending {pending:vec![handover_ipc::NativePendingPeer {id:"pending-test".into(),name:"New phone".into(),code:"12345678".into()}]},
                            Method::ContactsList=>ServerPayload::Contacts {contacts:vec![Contact {device_id:device(true).id,local_id:"alex".into(),display_name:"Alex Example".into(),phones:vec!["+1 312 555 0100".into()],emails:vec![],photo:Some(PHOTO.into())}]},
                            Method::TransfersList=>ServerPayload::TransferActivity {activities:transfer_log.lock().unwrap().clone()},
                            method=>{
                                mutations.send(method.clone()).unwrap();
                                match method {
                                    Method::AudioConfigure {change}=>{
                                        if reject.swap(false,Ordering::SeqCst) {ServerPayload::Error {code:handover_ipc::ErrorCode::BackendRejected,message:"fixture rejection".into(),conversation_id:None}}
                                        else {let mut current=audio.lock().unwrap();if let Some(source)=change.source {current.settings.source=source;}
                                        if let Some(output)=change.output {current.settings.output=output;}
                                        if let Some(playback)=change.playback {current.settings.playback=playback;}current.revision+=1;let snapshot=current.clone();let _=events.send(ServerPayload::AudioControlsUpdated {snapshot:snapshot.clone()});ServerPayload::AudioControls {snapshot}}
                                    },
                                    Method::AudioProfile {profile,..}=>{let mut stream=audio_stream.lock().unwrap();stream.profile=Some(profile);stream.revision+=1;ServerPayload::NativeAccepted},
                                    Method::AudioStart {..}|Method::AudioStop {..}=>ServerPayload::NativeAccepted,
                                    Method::CustomRun {name}=>ServerPayload::CustomResult {result:CustomCommandResult {name,accepted:true,exit_code:Some(0),failure:None}},
                                    Method::RemoteInputSend {text:Some(text),..} if text=="lost input"=>return,
                                    Method::ClipboardMirror {enable}=>{mirror.store(enable,Ordering::SeqCst);ServerPayload::NativeAccepted},
                                    Method::CallsControl {..}=>ServerPayload::CallQueued {request_id:"fixture-call".into()},
                                    Method::NativeFilesystemList {path,..}=>{let _=events.send(ServerPayload::Filesystem {result:FilesystemResult {device_id:device(true).id,request_id:"fixture-folder".into(),path:path.clone(),entries:if path=="." {vec![FilesystemEntry {name:"Downloads".into(),directory:true,size:None},FilesystemEntry {name:"readme.txt".into(),directory:false,size:Some(120)}]}else{vec![FilesystemEntry {name:"photo.jpg".into(),directory:false,size:Some(240)}]},failure:None}});ServerPayload::NativeAccepted},
                                    Method::FileDownload {device_id,path}=>{
                                        let sequence=download_counter.fetch_add(1,Ordering::SeqCst)+1;
                                        let download=FileDownload {device_id,transfer_id:format!("fixture-download-{sequence}"),source_path:path,state:FileDownloadState::Queued};
                                        let mut log=transfer_log.lock().unwrap();
                                        log.retain(|activity| !matches!(&activity.event,TransferActivityEvent::Download {download:old} if old.device_id==download.device_id && old.transfer_id==download.transfer_id));
                                        log.insert(0,TransferActivity {sequence:2,observed_at_ms:1_800_000_000_002,event:TransferActivityEvent::Download {download:download.clone()}});
                                        let payload=ServerPayload::FileDownload {download};
                                        let _=events.send(payload.clone());
                                        payload
                                    },
                                    Method::NotificationDismiss {notification_id}=>{let _=events.send(ServerPayload::NotificationRemoved {notification_id:notification_id.clone()});ServerPayload::CommandCompleted {notification_id}},
                                    Method::NotificationInvokeAction {notification_id,action_id}=>{assert_eq!(action_id,"fixture-action");ServerPayload::CommandCompleted {notification_id}},
                                    Method::NotificationReply {text,..} if text=="lost reply"=>return,
                                    Method::NotificationReply {text,..} if text=="rejected reply"=>ServerPayload::Error {code:handover_ipc::ErrorCode::NotificationNotFound,message:"fixture rejection".into(),conversation_id:None},
                                    Method::NotificationReply {notification_id,..}=>ServerPayload::CommandCompleted {notification_id},
                                    Method::ClipboardHistoryPin {id,pinned}=>{history.lock().unwrap().iter_mut().find(|e|e.id==id).unwrap().pinned=pinned;ServerPayload::NativeAccepted},
                                    Method::ClipboardHistoryClear {include_pinned}=>{history.lock().unwrap().retain(|e|e.pinned&&!include_pinned);ServerPayload::NativeAccepted},
                                    Method::MediaControl {command}=>ServerPayload::MediaAccepted {id:command.id().clone()},
                                    Method::ShareFile {file_url,..} if file_url.ends_with("unknown.txt")=>return,
                                    Method::ShareFile {device_id,file_url}=>ServerPayload::ShareAccepted {device_id,transfer_id:Some(if file_url.ends_with("example.cc"){"code-transfer"}else{"photo-transfer"}.into())},
                                    Method::ShareUrl {device_id,..}=>ServerPayload::ShareAccepted {device_id,transfer_id:Some("fixture-share".into())},
                                    Method::ClipboardSend {text,..} if text=="slow command"=>{gate.notified().await;ServerPayload::NativeAccepted},
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
            && has_text(&ui.phone_pane, "81%")
    });
    assert!(has_text(&ui.phone_pane, "81%"));
    assert!(received.try_recv().is_err());
    assert_eq!(*ui.selected.borrow(), Some(device(true).id));
    assert!(
        !ui.status_bar.is_visible(),
        "a healthy connection needs no permanent banner"
    );
    // Device actions are explicit requests, and acceptance does not invent device state.
    assert_eq!(ui.page.get(), Page::Overview);
    button(&ui.body, "Ring").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert_eq!(
        received.try_recv().unwrap(),
        Method::NativeRing {
            id: "phone-a".into()
        }
    );
    assert!(ui.model.borrow().notice.contains("queued"));
    // Remote input keeps drafts on lost acknowledgements and never sends on expand.
    expand(&ui.body, "Remote input");
    assert!(received.try_recv().is_err());
    let remote = descendants(&ui.body)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Entry>().ok())
        .unwrap();
    remote.set_text(&"🦀".repeat(200));
    assert!(
        !button(&ui.body, "Type text").is_sensitive(),
        "byte limit must hold for Unicode too"
    );
    remote.set_text("hello phone");
    button(&ui.body, "Type text").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert!(
        matches!(received.try_recv().unwrap(),Method::RemoteInputSend {device_id,action:RemoteInputAction::Type,text:Some(text),..} if device_id==device(true).id && text=="hello phone")
    );
    assert!(ui.remote_drafts.borrow().is_empty());
    let remote = descendants(&ui.body)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Entry>().ok())
        .unwrap();
    remote.set_text("lost input");
    button(&ui.body, "Type text").emit_clicked();
    spin(|| {
        ui.pending.borrow().is_empty() && ui.model.borrow().notice.contains("outcome is unknown")
    });
    assert!(
        matches!(received.try_recv().unwrap(),Method::RemoteInputSend {text:Some(text),..} if text=="lost input")
    );
    assert_eq!(ui.remote_drafts.borrow()[&device(true).id], "lost input");
    let remote = descendants(&ui.body)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Entry>().ok())
        .unwrap();
    remote.set_text("");
    button(&ui.body, "Left click").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert!(matches!(
        received.try_recv().unwrap(),
        Method::RemoteInputSend {
            action: RemoteInputAction::Click,
            button: 1,
            ..
        }
    ));
    ui.navigate(Page::Connection);
    spin(|| {
        ui.pending.borrow().is_empty()
            && ui.model.borrow().commands.is_some()
            && !ui.scheduled.get()
    });
    expand(&ui.body, "Phone connection test");
    button(&ui.body, "Ping phone").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert_eq!(
        received.try_recv().unwrap(),
        Method::NativePing {
            id: "phone-a".into()
        }
    );
    expand(&ui.body, "Desktop behavior");
    button(&ui.body, "Automatic").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert_eq!(received.try_recv().unwrap(), Method::ScreensaverFollow);
    button(&ui.body, "Keep desktop awake").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert_eq!(received.try_recv().unwrap(), Method::ScreensaverInhibit);
    button(&ui.body, "Allow desktop sleep").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert_eq!(received.try_recv().unwrap(), Method::ScreensaverRelease);
    expand(&ui.body, "Configured commands");
    button(&ui.body, "Run").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && has_text(&ui.body, "Exit 0"));
    assert_eq!(
        received.try_recv().unwrap(),
        Method::CustomRun {
            name: "fixture-command".into()
        }
    );
    ui.navigate(Page::Calls);
    expand(&ui.body, "Call audio on this computer");
    spin(|| ui.pending.borrow().is_empty() && has_text(&ui.body, "HFP profile selected"));
    assert!(has_text(&ui.body, "Incoming call"));
    assert!(
        !descendants(&ui.body)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Button>().ok())
            .any(|b| b.label().as_deref() == Some("Call"))
    );
    button(&ui.body, "Answer").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert_eq!(
        received.try_recv().unwrap(),
        Method::CallsControl {
            device_id: device(true).id,
            action: CallAction::Answer,
            address: None,
            expected_generation: Some(7)
        }
    );
    assert!(
        has_text(&ui.body, "Incoming call"),
        "queued answer does not claim a call connected"
    );
    ui.event(Event::State(Box::new(ServerPayload::CallUpdated {
        call: CallState {
            device_id: device(true).id,
            phase: CallPhase::Idle,
            controls: BTreeSet::from([CallAction::Place]),
            generation: 8,
        },
    })));
    spin(|| !ui.scheduled.get());
    assert!(has_text(&ui.body, "No active call"));
    let dial = descendants(&ui.body)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Entry>().ok())
        .unwrap();
    dial.set_text("*123#");
    assert!(!button(&ui.body, "Call").is_sensitive());
    dial.set_text("+12025550111");
    button(&ui.body, "Call").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert_eq!(
        received.try_recv().unwrap(),
        Method::CallsControl {
            device_id: device(true).id,
            action: CallAction::Place,
            address: Some("+12025550111".into()),
            expected_generation: Some(8)
        }
    );
    ui.navigate(Page::Sharing);
    button(&ui.body, "On phone").emit_clicked();
    assert_eq!(ui.page.get(), Page::PhoneFiles);
    assert!(
        received.try_recv().is_err(),
        "opening phone folders does not submit a request"
    );
    button(&ui.body, "Open folder").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && has_text(&ui.body, "readme.txt"));
    assert_eq!(
        received.try_recv().unwrap(),
        Method::NativeFilesystemList {
            id: "phone-a".into(),
            path: ".".into()
        }
    );
    button(&ui.body, "Open").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && has_text(&ui.body, "photo.jpg"));
    assert_eq!(
        received.try_recv().unwrap(),
        Method::NativeFilesystemList {
            id: "phone-a".into(),
            path: "Downloads".into()
        }
    );
    button(&ui.body, "Download").emit_clicked();
    spin(|| {
        ui.model.borrow().transfers.iter().any(|activity| matches!(
            &activity.event,
            TransferActivityEvent::Download { download } if matches!(download.state, FileDownloadState::Queued)
        ))
    });
    assert_eq!(
        received.try_recv().unwrap(),
        Method::FileDownload {
            device_id: device(true).id,
            path: "Downloads/photo.jpg".into(),
        }
    );
    let publish_download = |state| {
        let transfer_id = match &transfer_log.lock().unwrap()[0].event {
            TransferActivityEvent::Download { download } => download.transfer_id.clone(),
            _ => unreachable!("fixture queued the active download"),
        };
        let download = FileDownload {
            device_id: device(true).id,
            transfer_id,
            source_path: "Downloads/photo.jpg".into(),
            state,
        };
        let mut log = transfer_log.lock().unwrap();
        log[0] = TransferActivity {
            sequence: 3,
            observed_at_ms: 1_800_000_000_003,
            event: TransferActivityEvent::Download {
                download: download.clone(),
            },
        };
        drop(log);
        events
            .send(ServerPayload::FileDownload { download })
            .unwrap();
    };
    publish_download(FileDownloadState::Receiving);
    spin(|| has_text(&ui.body, "Receiving…"));
    publish_download(FileDownloadState::Completed {
        path: "/tmp/handover-fixture/photo.jpg".into(),
    });
    spin(|| has_text(&ui.body, "Download again"));
    assert!(has_text(&ui.body, "Saved"));
    ui.navigate(Page::Sharing);
    spin(|| !ui.scheduled.get());
    assert!(has_text(&ui.body, "Download saved"));
    assert!(has_text(&ui.body, "photo.jpg"));
    assert!(has_text(&ui.body, "Pixel test"));
    ui.navigate(Page::PhoneFiles);
    spin(|| !ui.scheduled.get());
    button(&ui.body, "Download again").emit_clicked();
    spin(|| {
        ui.model.borrow().transfers.iter().any(|activity| matches!(
        &activity.event,
        TransferActivityEvent::Download { download } if matches!(download.state, FileDownloadState::Queued)
    ))
    });
    assert_eq!(
        received.try_recv().unwrap(),
        Method::FileDownload {
            device_id: device(true).id,
            path: "Downloads/photo.jpg".into(),
        }
    );
    publish_download(FileDownloadState::Failed {
        reason: FileDownloadFailure::PermissionRequired,
    });
    spin(|| has_text(&ui.body, "Choose a shared folder in the phone app"));
    ui.navigate(Page::Sharing);
    spin(|| !ui.scheduled.get());
    assert!(has_text(
        &ui.body,
        "Choose a shared folder in the phone app"
    ));
    ui.navigate(Page::PhoneFiles);
    spin(|| !ui.scheduled.get());
    button(&ui.body, "Retry").emit_clicked();
    spin(|| {
        ui.model.borrow().transfers.iter().any(|activity| matches!(
        &activity.event,
        TransferActivityEvent::Download { download } if matches!(download.state, FileDownloadState::Queued)
    ))
    });
    assert_eq!(
        received.try_recv().unwrap(),
        Method::FileDownload {
            device_id: device(true).id,
            path: "Downloads/photo.jpg".into(),
        }
    );
    ui.event(Event::State(Box::new(ServerPayload::Filesystem {
        result: FilesystemResult {
            device_id: device(false).id,
            request_id: "unrelated".into(),
            path: "Downloads".into(),
            entries: vec![],
            failure: None,
        },
    })));
    assert_eq!(
        ui.model.borrow().folder.as_ref().unwrap().device_id,
        device(true).id
    );
    ui.model.borrow_mut().notice.clear();
    ui.navigate(Page::Notifications);
    spin(|| !ui.scheduled.get());
    // Reply drafts live independently of rebuilt notification rows and never send on open.
    button(&ui.body, "Reply…").emit_clicked();
    let editor = ui.reply_editor.borrow().clone().unwrap();
    editor.input.buffer().set_text("Reply with a\nsecond line");
    assert!(received.try_recv().is_err());
    let mut disconnected_phone = device(true);
    disconnected_phone.connected = false;
    ui.event(Event::State(Box::new(ServerPayload::DeviceUpdated {
        device: disconnected_phone,
    })));
    spin(|| !ui.scheduled.get());
    assert!(!editor.send.is_sensitive());
    assert_eq!(
        ui.reply_drafts.borrow()[&note(true).id].text,
        "Reply with a\nsecond line"
    );
    ui.event(Event::State(Box::new(ServerPayload::DeviceUpdated {
        device: device(true),
    })));
    spin(|| !ui.scheduled.get());
    assert!(editor.send.is_sensitive());
    ui.event(Event::State(Box::new(ServerPayload::NotificationUpdated {
        notification: note(true),
    })));
    spin(|| !ui.scheduled.get());
    assert_eq!(
        ui.reply_editor.borrow().as_ref().unwrap().input,
        editor.input
    );
    editor.window.close();
    ui.window.close();
    let close_dialog = app
        .windows()
        .into_iter()
        .find_map(|w| w.downcast::<adw::MessageDialog>().ok())
        .unwrap();
    close_dialog.response("keep");
    assert!(!ui.close_confirmed.get());
    assert!(ui.window.is_visible());
    assert!(!ui.reply_drafts.borrow().is_empty());
    button(&ui.body, "Reply…").emit_clicked();
    let editor = ui.reply_editor.borrow().clone().unwrap();
    let buffer = editor.input.buffer();
    assert_eq!(
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), false),
        "Reply with a\nsecond line"
    );
    editor.send.emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && ui.reply_editor.borrow().is_none());
    assert_eq!(
        received.try_recv().unwrap(),
        Method::NotificationReply {
            notification_id: note(true).id,
            text: "Reply with a\nsecond line".into()
        }
    );
    assert!(ui.reply_drafts.borrow().is_empty());
    assert!(
        ui.model.borrow().notifications.contains_key(&note(true).id),
        "acceptance does not dismiss the notification"
    );
    button(&ui.body, "Reply…").emit_clicked();
    let editor = ui.reply_editor.borrow().clone().unwrap();
    editor.input.buffer().set_text("rejected reply");
    editor.send.emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && has_text(&editor.window, "no longer available"));
    assert!(
        matches!(received.try_recv().unwrap(),Method::NotificationReply {text,..} if text=="rejected reply")
    );
    assert_eq!(
        ui.reply_drafts.borrow()[&note(true).id].text,
        "rejected reply"
    );
    editor.input.buffer().set_text("lost reply");
    editor.send.emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && has_text(&editor.window, "outcome is unknown"));
    assert!(
        matches!(received.try_recv().unwrap(), Method::NotificationReply {text,..} if text=="lost reply")
    );
    assert_eq!(ui.reply_drafts.borrow()[&note(true).id].text, "lost reply");
    editor.window.close();
    button(&ui.body, "Reply…").emit_clicked();
    let editor = ui.reply_editor.borrow().clone().unwrap();
    assert!(has_text(&editor.window, "outcome is unknown"));
    ui.event(Event::State(Box::new(ServerPayload::NotificationRemoved {
        notification_id: note(true).id,
    })));
    spin(|| !ui.scheduled.get());
    assert!(
        !editor.send.is_sensitive(),
        "removed notifications cannot receive a saved draft"
    );
    assert!(
        received.try_recv().is_err(),
        "reopening or receiving updates must not resend"
    );
    editor.window.close();
    spin(|| !ui.scheduled.get());
    assert!(has_text(&ui.body, "Reply drafts (1)"));
    let draft_menu = descendants(&ui.body)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::MenuButton>().ok())
        .unwrap();
    let popover = draft_menu.popover().unwrap();
    draft_menu.popup();
    button(&popover, "Reply draft for Unavailable notification").emit_clicked();
    let editor = ui.reply_editor.borrow().clone().unwrap();
    assert!(!editor.send.is_sensitive());
    assert!(has_text(&editor.window, "outcome is unknown"));
    button(&editor.window, "Discard draft").emit_clicked();
    assert!(ui.reply_drafts.borrow().is_empty());
    ui.event(Event::State(Box::new(ServerPayload::NotificationAdded {
        notification: note(true),
    })));
    spin(|| !ui.scheduled.get());
    ui.model.borrow_mut().notice.clear();
    ui.render();
    // Clipboard history is host-wide. Pin and clear reflect fresh daemon reads.
    ui.navigate(Page::Clipboard);
    spin(|| ui.pending.borrow().is_empty() && has_text(&ui.body, "Fictional clipboard entry"));
    assert!(has_text(&ui.body, "All devices"));
    assert!(has_text(&ui.body, "Clipboard mirroring is off"));
    button(&ui.body, "Turn on").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && has_text(&ui.body, "Clipboard mirroring is on"));
    assert_eq!(
        received.try_recv().unwrap(),
        Method::ClipboardMirror { enable: true }
    );
    button(&ui.body, "Turn off").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && has_text(&ui.body, "Clipboard mirroring is off"));
    assert_eq!(
        received.try_recv().unwrap(),
        Method::ClipboardMirror { enable: false }
    );
    let row = button(&ui.body, "Pin").parent().unwrap();
    button(&row, "Copy").emit_clicked();
    let (copied, copied_rx) = mpsc::channel();
    ui.window
        .clipboard()
        .read_text_async(None::<&gio::Cancellable>, move |result| {
            copied.send(result.unwrap().unwrap().to_string()).unwrap();
        });
    let mut value = None;
    spin(|| {
        if let Ok(text) = copied_rx.try_recv() {
            value = Some(text);
        }
        value.is_some()
    });
    assert_eq!(value.unwrap(), "Fictional clipboard entry");
    assert!(
        received.try_recv().is_err(),
        "copying does not send to a phone"
    );
    button(&ui.body, "Pin").emit_clicked();
    spin(|| {
        ui.pending.borrow().is_empty()
            && ui
                .model
                .borrow()
                .clipboard_history
                .as_ref()
                .unwrap()
                .iter()
                .all(|e| e.pinned)
    });
    assert_eq!(
        received.try_recv().unwrap(),
        Method::ClipboardHistoryPin {
            id: 1,
            pinned: true
        }
    );
    ui.search.set_text("Fictional");
    spin(|| !ui.scheduled.get() && !has_text(&ui.body, "Pinned clipboard entry"));
    button(&ui.body, "Unpin").emit_clicked();
    spin(|| {
        ui.pending.borrow().is_empty()
            && !ui
                .model
                .borrow()
                .clipboard_history
                .as_ref()
                .unwrap()
                .iter()
                .find(|e| e.id == 1)
                .unwrap()
                .pinned
    });
    assert_eq!(
        received.try_recv().unwrap(),
        Method::ClipboardHistoryPin {
            id: 1,
            pinned: false
        }
    );
    ui.search.set_text("");
    spin(|| !ui.scheduled.get() && has_text(&ui.body, "Pinned clipboard entry"));
    button(&ui.body, "Clear history…").emit_clicked();
    let dialog = app
        .windows()
        .into_iter()
        .find_map(|w| w.downcast::<adw::MessageDialog>().ok())
        .unwrap();
    assert!(received.try_recv().is_err());
    dialog.response("cancel");
    assert_eq!(history.lock().unwrap().len(), 2);
    button(&ui.body, "Clear history…").emit_clicked();
    let dialog = app
        .windows()
        .into_iter()
        .find_map(|w| w.downcast::<adw::MessageDialog>().ok())
        .unwrap();
    dialog.response("clear");
    spin(|| {
        ui.pending.borrow().is_empty()
            && ui.model.borrow().clipboard_history.as_ref().unwrap().len() == 1
    });
    assert_eq!(
        received.try_recv().unwrap(),
        Method::ClipboardHistoryClear {
            include_pinned: false
        }
    );
    assert_eq!(history.lock().unwrap()[0].id, 2);
    button(&ui.body, "Clear history…").emit_clicked();
    let dialog = app
        .windows()
        .into_iter()
        .find_map(|w| w.downcast::<adw::MessageDialog>().ok())
        .unwrap();
    descendants(&dialog)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::CheckButton>().ok())
        .unwrap()
        .set_active(true);
    dialog.response("clear");
    spin(|| ui.pending.borrow().is_empty() && has_text(&ui.body, "No clipboard history"));
    assert_eq!(
        received.try_recv().unwrap(),
        Method::ClipboardHistoryClear {
            include_pinned: true
        }
    );
    ui.navigate(Page::Notifications);
    spin(|| !ui.scheduled.get());
    // Actions are gated by actual device connection, not the presence of an old notification.
    ui.selected.borrow_mut().take();
    ui.page.set(Page::Notifications);
    ui.render();
    // Battery updates must not rebuild a notification list or steal its keyboard focus.
    let notification_group = ui.body.first_child().unwrap();
    let rail_row = ui.devices.row_at_index(1).unwrap();
    let mut changed = device(true);
    changed.battery = Some(BatteryState::new(80, true).unwrap());
    events
        .send(ServerPayload::DeviceUpdated { device: changed })
        .unwrap();
    spin(|| {
        ui.model.borrow().devices[&device(true).id]
            .battery
            .unwrap()
            .percentage()
            == 80
            && !ui.scheduled.get()
    });
    assert_eq!(ui.body.first_child().unwrap(), notification_group);
    assert_eq!(ui.devices.row_at_index(1).unwrap(), rail_row);
    let next_track = button(&ui.body, "Next track");
    if std::env::var_os("HANDOVER_CONTROL_TEST_BLACK_ACCENT").is_some() {
        let label = descendants(&next_track)
            .into_iter()
            .find_map(|widget| widget.downcast::<gtk::Label>().ok())
            .expect("notification action button has a label");
        let foreground = label.color();
        let luminance =
            0.2126 * foreground.red() + 0.7152 * foreground.green() + 0.0722 * foreground.blue();
        assert!(
            luminance > 0.5,
            "notification action label must contrast with the dark surface, got {foreground:?}"
        );
    }
    next_track.emit_clicked();
    spin(|| ui.pending.borrow().is_empty());
    assert_eq!(
        received.try_recv().unwrap(),
        Method::NotificationInvokeAction {
            notification_id: note(true).id,
            action_id: "fixture-action".into(),
        }
    );
    let buttons = descendants(&ui.body)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Button>().ok())
        .filter(|b| {
            b.label().as_deref() == Some("Dismiss on phone")
                || b.tooltip_text().as_deref() == Some("Dismiss on phone")
        })
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
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert_eq!(
        received.try_recv().unwrap(),
        Method::NativePair {
            id: "pending-test".into(),
            code: "12345678".into()
        }
    );
    // A task tab changes the main view without losing the selected phone.
    ui.devices.select_row(ui.devices.row_at_index(1).as_ref());
    ui.tabs
        .iter()
        .find(|(page, _)| *page == Page::Contacts)
        .unwrap()
        .1
        .set_active(true);
    assert_eq!(ui.page.get(), Page::Contacts);
    assert_eq!(*ui.selected.borrow(), Some(device(true).id));
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
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert_eq!(
        received.try_recv().unwrap(),
        Method::ShareUrl {
            device_id: device(true).id,
            url: "https://example.test/shared".into()
        }
    );
    assert!(ui.model.borrow().notice.contains("accepted"));
    assert!(has_text(&ui.body, "Received by phone"));
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
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert!(
        matches!(received.try_recv().unwrap(),Method::ClipboardSend {text,..} if text=="Fictional clipboard text")
    );
    assert!(ui.model.borrow().notice.contains("outcome is unknown"));
    // Dropping on the window opens Files for review. It never sends until the explicit Send action.
    ui.navigate(Page::Notifications);
    let code_path = root.join("example.cc");
    std::fs::write(&code_path, "int main() { return 0; }\n").unwrap();
    let photo_path = root.join("photo.png");
    std::fs::write(&photo_path, b"fixture-file").unwrap();
    let chosen = [
        gio::File::for_path(&code_path),
        gio::File::for_path(&photo_path),
    ];
    let controllers = ui.window.observe_controllers();
    let drop_target = (0..controllers.n_items())
        .find_map(|n| controllers.item(n).and_downcast::<gtk::DropTarget>())
        .unwrap();
    let payload = glib::BoxedValue(gtk::gdk::FileList::from_array(&chosen).to_value()).to_value();
    assert!(drop_target.emit_by_name::<bool>("drop", &[&payload, &0.0f64, &0.0f64]));
    spin(|| has_text(&ui.body, "example.cc"));
    assert_eq!(ui.page.get(), Page::Sharing);
    assert_eq!(ui.file_queue.borrow().len(), 2);
    assert!(received.try_recv().is_err());
    // Recipient changes preserve the selection and gate Send by actual capabilities and connection.
    ui.devices.select_row(ui.devices.row_at_index(2).as_ref());
    assert!(!button(&ui.body, "Send selected files").is_sensitive());
    assert_eq!(ui.file_queue.borrow().len(), 2);
    ui.devices.select_row(ui.devices.row_at_index(1).as_ref());
    button(&ui.body, "Send selected files").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    let mut sent = Vec::new();
    for _ in 0..2 {
        match received.try_recv().unwrap() {
            Method::ShareFile {
                device_id,
                file_url,
            } => {
                assert_eq!(device_id, device(true).id);
                sent.push(file_url);
            }
            other => panic!("{other:?}"),
        }
    }
    assert!(sent.contains(&chosen[0].uri().to_string()));
    assert!(sent.contains(&chosen[1].uri().to_string()));
    assert!(
        ui.file_queue
            .borrow()
            .iter()
            .all(|file| matches!(file.status, files::FileStatus::Accepted { .. })),
        "command acceptance must not claim receipt"
    );
    assert_eq!(
        std::fs::read_to_string(&code_path).unwrap(),
        "int main() { return 0; }\n"
    );
    events
        .send(ServerPayload::ShareResult {
            result: ShareResult {
                device_id: device(true).id,
                transfer_id: "code-transfer".into(),
                status: ShareStatus::Completed,
                reason: None,
            },
        })
        .unwrap();
    events
        .send(ServerPayload::ShareResult {
            result: ShareResult {
                device_id: device(true).id,
                transfer_id: "photo-transfer".into(),
                status: ShareStatus::Failed,
                reason: Some(ShareFailure::Storage),
            },
        })
        .unwrap();
    spin(|| {
        matches!(
            ui.file_queue.borrow()[0].status,
            files::FileStatus::Received { .. }
        ) && matches!(
            ui.file_queue.borrow()[1].status,
            files::FileStatus::Failed { .. }
        )
    });
    assert!(!button(&ui.body, "Send selected files").is_sensitive());
    // Duplicate drops and oversized selections do not add files or send partial selections.
    assert!(!ui.add_files(chosen.to_vec()));
    assert_eq!(ui.file_queue.borrow().len(), 2);
    let oversized = (0..9)
        .map(|n| gio::File::for_path(root.join(format!("extra-{n}.txt"))))
        .collect();
    assert!(!ui.add_files(oversized));
    assert_eq!(ui.file_queue.borrow().len(), 2);
    assert!(received.try_recv().is_err());
    // Closing an unsent selection offers Keep open rather than silently losing it.
    let unsent = gio::File::for_path(root.join("unsent.txt"));
    std::fs::write(root.join("unsent.txt"), "not sent").unwrap();
    assert!(ui.add_files(vec![unsent.clone()]));
    ui.window.close();
    assert!(ui.close_prompt.get());
    let prompt = app
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<adw::MessageDialog>().ok())
        .unwrap();
    prompt.response("keep");
    assert!(!prompt.is_visible());
    assert!(ui.window.is_visible());
    assert_eq!(ui.file_queue.borrow().len(), 3);
    let name = descendants(&ui.body)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Label>().ok())
        .find(|label| label.text() == "unsent.txt")
        .unwrap();
    let row = name.parent().unwrap().parent().unwrap();
    button(
        &row,
        "Remove from list; submitted transfers are not cancelled",
    )
    .emit_clicked();
    assert_eq!(ui.file_queue.borrow().len(), 2);
    // A consumed file request with a lost reply stays unconfirmed and is not automatically resent.
    let unknown = gio::File::for_path(root.join("unknown.txt"));
    std::fs::write(root.join("unknown.txt"), "uncertain fixture").unwrap();
    assert!(ui.add_files(vec![unknown.clone()]));
    button(&ui.body, "Send selected files").emit_clicked();
    spin(|| {
        ui.pending.borrow().is_empty()
            && matches!(
                ui.file_queue.borrow()[2].status,
                files::FileStatus::Unconfirmed { .. }
            )
    });
    assert!(
        matches!(received.try_recv().unwrap(),Method::ShareFile {file_url,..} if file_url==unknown.uri())
    );
    assert!(received.try_recv().is_err());
    assert!(!button(&ui.body, "Send selected files").is_sensitive());
    let name = descendants(&ui.body)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Label>().ok())
        .find(|label| label.text() == "unknown.txt")
        .unwrap();
    let row = name.parent().unwrap().parent().unwrap();
    button(
        &row,
        "Remove from list; submitted transfers are not cancelled",
    )
    .emit_clicked();
    assert_eq!(ui.file_queue.borrow().len(), 2);
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
    spin(|| {
        descendants(&ui.body)
            .into_iter()
            .filter_map(|w| w.downcast::<adw::Avatar>().ok())
            .any(|a| a.custom_image().is_some())
    });
    let photo = descendants(&ui.body)
        .into_iter()
        .find_map(|w| w.downcast::<adw::Avatar>().ok())
        .unwrap()
        .custom_image()
        .unwrap();
    ui.render();
    let rebuilt = descendants(&ui.body)
        .into_iter()
        .find_map(|w| w.downcast::<adw::Avatar>().ok())
        .unwrap();
    assert_eq!(
        rebuilt.custom_image().as_ref(),
        Some(&photo),
        "cached pictures must be installed immediately on rebuild"
    );
    button(&ui.body, "Call…").emit_clicked();
    assert_eq!(ui.page.get(), Page::Calls);
    assert_eq!(*ui.dial_address.borrow(), "+13125550100");
    assert!(
        received.try_recv().is_err(),
        "contact shortcut only opens the dialer"
    );
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    // Failed routing inspection must not hide receiver controls or the independent phone player.
    read_audio_available.store(false, Ordering::SeqCst);
    ui.navigate(Page::Media);
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert!(ui.model.borrow().audio_controls.is_none());
    assert!(button(&ui.body, "Start laptop audio").is_sensitive());
    assert!(has_text(&ui.body, "Laptop routing controls unavailable"));
    read_audio_available.store(true, Ordering::SeqCst);
    ui.refresh.emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert!(ui.model.borrow().audio_controls.is_some());
    assert!(ui.model.borrow().audio_error.is_none());
    // Show the control appropriate to the reported playback state, without optimistic changes.
    assert!(has_text(&ui.body, "Evening walk"));
    assert!(
        !descendants(&ui.body)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Button>().ok())
            .any(|b| b.tooltip_text().as_deref() == Some("Play"))
    );
    button(&ui.body, "Pause").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert!(matches!(
        received.try_recv().unwrap(),
        Method::MediaControl {
            command: MediaCommand::Pause { .. }
        }
    ));
    assert_eq!(
        ui.model.borrow().media.values().next().unwrap().playback,
        PlaybackState::Playing
    );
    // Audio controls report receiver state; accepted intent alone never claims streaming.
    spin(|| {
        ui.pending.borrow().is_empty()
            && ui.model.borrow().audio_controls.is_some()
            && !ui.scheduled.get()
    });
    button(&ui.body, "Start laptop audio").emit_clicked();
    assert_eq!(
        ui.model.borrow().audio_streams[&device(true).id].state,
        AudioStreamPhase::Idle
    );
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert_eq!(
        received.try_recv().unwrap(),
        Method::AudioStart {
            device_id: device(true).id
        }
    );
    assert!(has_text(&ui.body, "Not streaming"));
    let mut streaming = audio_stream.lock().unwrap().clone();
    streaming.state = AudioStreamPhase::Streaming;
    streaming.enabled = Some(true);
    streaming.revision += 1;
    *audio_stream.lock().unwrap() = streaming.clone();
    events
        .send(ServerPayload::AudioStreamUpdated { status: streaming })
        .unwrap();
    spin(|| has_text(&ui.body, "Receiver output started"));
    assert!(button(&ui.body, "Stop laptop audio").is_sensitive());
    button(&ui.body, "Music").emit_clicked();
    spin(|| {
        ui.pending.borrow().is_empty()
            && ui.model.borrow().audio_streams[&device(true).id].profile
                == Some(AudioProfile::Music)
    });
    assert_eq!(
        received.try_recv().unwrap(),
        Method::AudioProfile {
            device_id: device(true).id,
            profile: AudioProfile::Music
        }
    );
    // Diagnostic reports update labels in place, preserving routing widgets and focus.
    expand(&ui.body, "Audio diagnostics");
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    let body_child = ui.body.first_child().unwrap();
    let mut measured = ui.model.borrow().audio_streams[&device(true).id].clone();
    measured.revision += 1;
    measured.diagnostics = Some(AudioStreamDiagnostics {
        sender: None,
        receiver: Some(AudioReceiverDiagnostics {
            route: AudioRoute::Direct,
            transport: Some(AudioTransport::Udp),
            round_trip_ms: Some(25),
            jitter_ms: Some(3),
            jitter_buffer_ms: Some(60),
            bitrate_bps: Some(64000),
            packets_received: Some(400),
            packets_lost: Some(2),
            updated_at_ms: 1_800_000_000_000,
        }),
    });
    *audio_stream.lock().unwrap() = measured.clone();
    events
        .send(ServerPayload::AudioStreamUpdated { status: measured })
        .unwrap();
    spin(|| has_text(&ui.body, "25 ms round trip"));
    assert_eq!(ui.body.first_child().unwrap(), body_child);
    button(&ui.body, "Stop laptop audio").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert_eq!(
        received.try_recv().unwrap(),
        Method::AudioStop {
            device_id: device(true).id
        }
    );
    assert_eq!(
        ui.model.borrow().audio_streams[&device(true).id].state,
        AudioStreamPhase::Streaming,
        "stop acknowledgement is not receiver confirmation"
    );
    let stale_stream = AudioStreamStatus::idle(device(true).id);
    ui.event(Event::State(Box::new(ServerPayload::AudioStreamUpdated {
        status: stale_stream,
    })));
    spin(|| !ui.scheduled.get());
    assert_eq!(
        ui.model.borrow().audio_streams[&device(true).id].state,
        AudioStreamPhase::Streaming
    );
    assert_eq!(
        ui.model.borrow().audio_streams[&device(true).id].profile,
        Some(AudioProfile::Music)
    );
    expand(&ui.body, "Laptop audio routing");
    reject_audio.store(true, Ordering::SeqCst);
    button(&ui.body, "Both").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert_eq!(
        ui.model
            .borrow()
            .audio_controls
            .as_ref()
            .unwrap()
            .settings
            .output,
        AudioOutputMode::Phone
    );
    assert!(matches!(
        received.try_recv().unwrap(),
        Method::AudioConfigure {
            change: AudioRoutingChange {
                output: Some(AudioOutputMode::Both),
                ..
            }
        }
    ));
    button(&ui.body, "Both").emit_clicked();
    spin(|| {
        ui.pending.borrow().is_empty()
            && ui
                .model
                .borrow()
                .audio_controls
                .as_ref()
                .unwrap()
                .settings
                .output
                == AudioOutputMode::Both
    });
    assert!(matches!(
        received.try_recv().unwrap(),
        Method::AudioConfigure { .. }
    ));
    check(&ui.body, "Selected applications").set_active(true);
    spin(|| !ui.scheduled.get() && has_text(&ui.body, "Select at least one"));
    assert!(!button(&ui.body, "Apply audio source").is_sensitive());
    check(&ui.body, "Browser").set_active(true);
    spin(|| !ui.scheduled.get());
    assert!(matches!(
        audio.lock().unwrap().settings.source,
        AudioSourceSelection::All
    ));
    assert!(received.try_recv().is_err());
    button(&ui.body, "Apply audio source").emit_clicked();
    spin(|| ui.pending.borrow().is_empty() && ui.audio_source_draft.borrow().is_none());
    assert!(
        matches!(received.try_recv().unwrap(),Method::AudioConfigure {change:AudioRoutingChange {source:Some(AudioSourceSelection::Applications {ids}),..}} if ids==vec!["browser"])
    );
    // Returning a draft to the reported value must not resurrect a discarded checkbox on the next edit.
    check(&ui.body, "Music application").set_active(true);
    spin(|| !ui.scheduled.get());
    check(&ui.body, "Music application").set_active(false);
    check(&ui.body, "Browser").set_active(false);
    spin(|| !ui.scheduled.get());
    assert!(
        matches!(&*ui.audio_source_draft.borrow(),Some(AudioSourceSelection::Applications {ids}) if ids.is_empty())
    );
    assert!(!button(&ui.body, "Apply audio source").is_sensitive());
    check(&ui.body, "Browser").set_active(true);
    spin(|| !ui.scheduled.get());
    assert!(ui.audio_source_draft.borrow().is_none());
    let picker = descendants(&ui.body)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::DropDown>().ok())
        .unwrap();
    picker.set_selected(1);
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert!(
        matches!(received.try_recv().unwrap(),Method::AudioConfigure {change:AudioRoutingChange {playback:Some(AudioPlaybackTarget::Player {id}),..}} if id=="player-1")
    );
    // A slow old read must not overwrite a newer audio-settings event.
    slow_audio.store(true, Ordering::SeqCst);
    ui.request(Request::Audio);
    spin(|| !slow_audio.load(Ordering::SeqCst));
    let mut newer = audio.lock().unwrap().clone();
    newer.revision += 10;
    newer.settings.output = AudioOutputMode::Laptop;
    *audio.lock().unwrap() = newer.clone();
    events
        .send(ServerPayload::AudioControlsUpdated {
            snapshot: newer.clone(),
        })
        .unwrap();
    spin(|| ui.model.borrow().audio_controls.as_ref().unwrap().revision == newer.revision);
    audio_gate.notify_one();
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    assert_eq!(
        ui.model.borrow().audio_controls.as_ref().unwrap().revision,
        newer.revision
    );
    assert_eq!(
        ui.model
            .borrow()
            .audio_controls
            .as_ref()
            .unwrap()
            .settings
            .output,
        AudioOutputMode::Laptop
    );
    // Unknown routing observations cannot appear as a selected default configuration.
    let mut unavailable = newer.clone();
    unavailable.revision += 1;
    unavailable.settings_available = false;
    events
        .send(ServerPayload::AudioControlsUpdated {
            snapshot: unavailable,
        })
        .unwrap();
    spin(|| has_text(&ui.body, "Laptop audio settings are unavailable"));
    assert!(
        !descendants(&ui.body)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Button>().ok())
            .any(|b| b.label().as_deref() == Some("Both"))
    );
    let mut restored = newer;
    restored.revision += 2;
    *audio.lock().unwrap() = restored.clone();
    events
        .send(ServerPayload::AudioControlsUpdated { snapshot: restored })
        .unwrap();
    spin(|| {
        !ui.scheduled.get()
            && ui
                .model
                .borrow()
                .audio_controls
                .as_ref()
                .unwrap()
                .settings_available
    });
    // A slow command reply must not block the subscription's live device updates.
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    ui.request(Request::Clipboard(device(true).id, "slow command".into()));
    let mut admitted = false;
    spin(|| {
        if let Ok(Method::ClipboardSend { text, .. }) = received.try_recv() {
            assert_eq!(text, "slow command");
            admitted = true;
        }
        admitted
    });
    let mut changed = device(true);
    changed.battery = Some(BatteryState::new(79, true).unwrap());
    events
        .send(ServerPayload::DeviceUpdated { device: changed })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline
        && ui.model.borrow().devices[&device(true).id]
            .battery
            .unwrap()
            .percentage()
            != 79
    {
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let responsive = ui.model.borrow().devices[&device(true).id]
        .battery
        .unwrap()
        .percentage()
        == 79;
    command_gate.notify_one();
    assert!(
        responsive,
        "live subscription updates were blocked by a pending command reply"
    );
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
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
    // Work waiting behind an in-flight command is rejected when its subscription is lost.
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    ui.request(Request::Clipboard(device(true).id, "slow command".into()));
    let mut admitted = false;
    spin(|| {
        if let Ok(Method::ClipboardSend { text, .. }) = received.try_recv() {
            assert_eq!(text, "slow command");
            admitted = true;
        }
        admitted
    });
    ui.request(Request::Clipboard(
        device(true).id,
        "stale queued command".into(),
    ));
    offline.store(true, Ordering::SeqCst);
    reset.send(1).unwrap();
    spin(|| !ui.model.borrow().connected);
    command_gate.notify_one();
    spin(|| ui.pending.borrow().is_empty() && !ui.scheduled.get());
    // Retrying a lost subscription can report Disconnected again after the command verdict.
    ui.event(Event::Disconnected);
    assert!(
        ui.model.borrow().notice.contains("Nothing was submitted"),
        "Operation outcome was lost: {:?}",
        ui.model.borrow().notice
    );
    assert!(
        received.try_recv().is_err(),
        "queued work must not start after session loss"
    );
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
        replacement.model.borrow().transfers == *transfer_log.lock().unwrap()
            && replacement.model.borrow().contacts.len() == 1
    });
    assert!(received.try_recv().is_err());
    replacement.window.close();
    let page = match std::env::var("HANDOVER_CONTROL_TEST_PAGE").as_deref() {
        Ok("overview") => Page::Overview,
        Ok("calls") => Page::Calls,
        Ok("phone-files") => Page::PhoneFiles,
        Ok("notifications") => Page::Notifications,
        Ok("sharing") => Page::Sharing,
        Ok("contacts") => Page::Contacts,
        Ok("clipboard") => Page::Clipboard,
        Ok("media") => Page::Media,
        Ok("connection") => Page::Connection,
        _ => Page::Notifications,
    };
    ui.navigate(page);
    if let Some(path) = std::env::var_os("HANDOVER_CONTROL_TEST_SCREENSHOT") {
        if ui.status_bar.is_visible() {
            button(&ui.status_bar, "Dismiss notice").emit_clicked();
        }
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
