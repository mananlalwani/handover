use super::*;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const ACCOUNT: &str = "gmessages:gui-test";
const THREAD: &str = "thread-1";

fn id() -> ConversationId {
    ConversationId::new(MessagingAccountId::new(ACCOUNT), THREAD)
}

fn fixture() -> (MessagingAccount, Conversation) {
    let account = MessagingAccount {
        id: MessagingAccountId::new(ACCOUNT),
        label: "Test account".into(),
        connected: true,
        authenticated: true,
    };
    let conversation = Conversation {
        id: id(),
        kind: ConversationKind::Direct,
        transport: TransportKind::Rcs,
        title: Some("Alex Example".into()),
        participants: vec![Participant {
            local_id: "alex".into(),
            display_name: Some("Alex Example".into()),
            address: None,
            is_self: false,
        }],
        latest_message_id: None,
        last_activity_at: Some(1),
        unread_count: Some(0),
        cursor: None,
        capabilities: [MessagingCapability::Text, MessagingCapability::Replies]
            .into_iter()
            .collect(),
    };
    (account, conversation)
}

fn guard() -> PathBuf {
    let root = PathBuf::from(
        std::env::var_os("HANDOVER_MESSAGES_TEST_ROOT")
            .expect("run scripts/test-handover-messages.py"),
    );
    let runtime = root.join("runtime");
    assert_eq!(
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        Some(runtime.clone())
    );
    runtime.join("handover/handoverd.sock")
}

fn send_json(stream: &mut UnixStream, value: Value) {
    let _ = writeln!(stream, "{}", value);
    let _ = stream.flush();
}

fn response(kind: &str, fields: Value) -> Value {
    let mut value = json!({"protocol": 1, "type": kind});
    if let (Some(dst), Some(src)) = (value.as_object_mut(), fields.as_object()) {
        dst.extend(src.clone());
    }
    value
}

struct FixtureState {
    fail_send: Arc<AtomicBool>,
    disconnect_subscription: Arc<AtomicBool>,
    subscriptions: Arc<std::sync::atomic::AtomicUsize>,
    subscription_writer: Arc<Mutex<Option<UnixStream>>>,
    requests: Arc<Mutex<Vec<Value>>>,
    sent: Arc<Mutex<Vec<String>>>,
    drafts: Arc<Mutex<std::collections::BTreeMap<String, Value>>>,
}

fn fake_server(path: PathBuf, state: FixtureState) -> thread::JoinHandle<()> {
    let FixtureState {
        fail_send,
        disconnect_subscription,
        subscriptions,
        subscription_writer,
        requests,
        sent,
        drafts,
    } = state;
    let listener = UnixListener::bind(path).unwrap();
    thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(stream) = connection else { break };
            let fail_send = fail_send.clone();
            let disconnect_subscription = disconnect_subscription.clone();
            let subscriptions = subscriptions.clone();
            let subscription_writer = subscription_writer.clone();
            let requests = requests.clone();
            let sent = sent.clone();
            let drafts = drafts.clone();
            thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() || line.is_empty() {
                    return;
                }
                let request: Value = serde_json::from_str(&line).unwrap();
                requests.lock().unwrap().push(request.clone());
                let method = request["method"].as_str().unwrap_or("");
                let mut stream = stream;
                let (account, conversation) = fixture();
                match method {
                    "subscribe" => {
                        subscriptions.fetch_add(1, Ordering::SeqCst);
                        disconnect_subscription.store(false, Ordering::SeqCst);
                        send_json(
                            &mut stream,
                            response(
                                "subscribed",
                                json!({"devices": [], "notifications": [], "media_sessions": [], "calls": [], "messaging_accounts": [account], "conversations": [conversation], "typing_states": [], "read_states": []}),
                            ),
                        );
                        *subscription_writer.lock().unwrap() = stream.try_clone().ok();
                        // Keep the subscription open until UI exits. The client uses it only for events.
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(150)));
                        while !disconnect_subscription.load(Ordering::SeqCst) {
                            let mut sink = String::new();
                            match reader.read_line(&mut sink) {
                                Ok(0) => break,
                                Ok(_) => {}
                                Err(error)
                                    if matches!(
                                        error.kind(),
                                        std::io::ErrorKind::WouldBlock
                                            | std::io::ErrorKind::TimedOut
                                    ) => {}
                                Err(_) => break,
                            }
                        }
                        subscription_writer.lock().unwrap().take();
                    }
                    "messages.workspace" | "messages.select" => send_json(
                        &mut stream,
                        response(
                            "messaging_workspace",
                            json!({"workspace": {"location": null, "draft_conversations": [], "persistent": true}}),
                        ),
                    ),
                    "messages.draft" => {
                        let conversation_id = request["conversation_id"].clone();
                        let key = conversation_id.to_string();
                        let draft = drafts.lock().unwrap().get(&key).cloned().unwrap_or_else(|| json!({"conversation_id":conversation_id,"text":"","reply_to":null,"revision":0}));
                        send_json(
                            &mut stream,
                            response("message_draft", json!({"draft":draft})),
                        );
                    }
                    "messages.save_draft" => {
                        let mut draft = request["draft"].clone();
                        draft["revision"] = json!(draft["revision"].as_u64().unwrap_or(0) + 1);
                        drafts
                            .lock()
                            .unwrap()
                            .insert(draft["conversation_id"].to_string(), draft.clone());
                        send_json(
                            &mut stream,
                            response("message_draft", json!({"draft":draft})),
                        );
                    }
                    "messages.history" => send_json(
                        &mut stream,
                        response(
                            "history",
                            json!({"conversation_id":request["conversation_id"],"messages":[],"cursor_next":null}),
                        ),
                    ),
                    "messages.search" => {
                        let message_id =
                            json!({"conversation_id":conversation.id,"local_id":"message-1"});
                        let hit = json!({"message_id":message_id,"snippet":"Found in saved history","sender":{"local_id":"alex","display_name":"Alex Example","address":null,"is_self":false},"sent_at":1});
                        send_json(
                            &mut stream,
                            response("message_search", json!({"hits":[hit],"has_more":false})),
                        );
                    }
                    "messages.context" => {
                        let message_id = request["message_id"].clone();
                        let message = json!({"id":message_id,"sender":{"local_id":"alex","display_name":"Alex Example","address":null,"is_self":false},"transport":"rcs","sent_at":1,"text":"Found in saved history","attachments":[],"reply_to":null,"reactions":[],"deleted":false});
                        send_json(
                            &mut stream,
                            response(
                                "message_context",
                                json!({"conversation_id":conversation.id,"messages":[message],"done":true}),
                            ),
                        );
                    }
                    "messages.send" => {
                        let text = request["text"].as_str().unwrap_or("").to_owned();
                        sent.lock().unwrap().push(text);
                        if fail_send.swap(false, Ordering::SeqCst) {
                            disconnect_subscription.store(true, Ordering::SeqCst);
                        } else {
                            send_json(
                                &mut stream,
                                response("message_accepted", json!({"request_id":"op-1"})),
                            );
                        }
                    }
                    "messages.read" => send_json(
                        &mut stream,
                        response(
                            "conversation_accepted",
                            json!({"conversation_id":conversation.id}),
                        ),
                    ),
                    _ => panic!("unexpected fake IPC request: {method}"),
                }
            });
        }
    })
}

fn pump_until(mut condition: impl FnMut() -> bool, description: &str) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(8));
    }
    panic!("timed out waiting for {description}");
}

fn text(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

fn find_widget(
    root: &gtk::Widget,
    predicate: &impl Fn(&gtk::Widget) -> bool,
) -> Option<gtk::Widget> {
    if predicate(root) {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = find_widget(&widget, predicate) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn setup_window(app: &adw::Application) -> adw::ApplicationWindow {
    pump_until(
        || {
            app.windows()
                .iter()
                .any(|window| window.title().as_deref() == Some("Connect Google Messages"))
        },
        "native setup window",
    );
    app.windows()
        .into_iter()
        .find(|window| window.title().as_deref() == Some("Connect Google Messages"))
        .and_downcast::<adw::ApplicationWindow>()
        .expect("setup application window")
}

struct ChildProcess(Child);

impl Drop for ChildProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn saw_target(requests: &Mutex<Vec<Value>>, method: &str, target: &str) -> bool {
    requests.lock().unwrap().iter().any(|request| {
        request["method"] == method
            && (request["conversation_id"]["local_id"] == target
                || request["location"]["conversation_id"]["local_id"] == target)
    })
}

#[test]
#[ignore = "run scripts/test-handover-messages.py"]
fn native_window_round_trips_real_frontend_actions() {
    let root = PathBuf::from(
        std::env::var_os("HANDOVER_MESSAGES_TEST_ROOT")
            .expect("run scripts/test-handover-messages.py"),
    );
    let runtime = root.join("runtime");
    assert_eq!(
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        Some(runtime)
    );
    let socket = guard();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let fail_send = Arc::new(AtomicBool::new(false));
    let disconnect_subscription = Arc::new(AtomicBool::new(false));
    let subscriptions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let subscription_writer = Arc::new(Mutex::new(None));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let drafts = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    let _server = fake_server(
        socket,
        FixtureState {
            fail_send: fail_send.clone(),
            disconnect_subscription,
            subscriptions: subscriptions.clone(),
            subscription_writer: subscription_writer.clone(),
            requests: requests.clone(),
            sent: sent.clone(),
            drafts,
        },
    );
    assert!(
        root.join("runtime/handover/handoverd.sock").exists(),
        "private fake daemon must be bound before constructing the window"
    );
    gtk::init().expect("GTK display");
    let app = adw::Application::builder()
        .application_id("org.handover.Messages.GuiTest")
        .build();
    app.register(None::<&gtk::gio::Cancellable>)
        .expect("register test application");
    let ui = Window::new(&app);
    ui.window.present();
    pump_until(|| ui.model.borrow().connected, "fake daemon connection");
    ui.open(id(), None);
    pump_until(
        || ui.model.borrow().selected.as_ref() == Some(&id()),
        "conversation selection",
    );
    pump_until(
        || ui.model.borrow().selected_draft().is_some_and(|d| d.loaded),
        "draft load",
    );

    ui.compose.buffer().set_text("persist this text");
    pump_until(
        || {
            ui.model
                .borrow()
                .selected_draft()
                .is_some_and(|d| !d.dirty && d.value.revision > 0)
        },
        "draft save through IPC",
    );
    ui.open(
        ConversationId::new(MessagingAccountId::new(ACCOUNT), "other"),
        None,
    );
    ui.open(id(), None);
    pump_until(
        || text(&ui.compose) == "persist this text",
        "restoring saved draft after switching conversations",
    );
    ui.search.set_text("saved term");
    pump_until(
        || !ui.model.borrow().hits.is_empty(),
        "search results from IPC",
    );
    let result_row = ui.list.row_at_index(0).expect("search hit row");
    ui.list.emit_by_name::<()>("row-activated", &[&result_row]);
    pump_until(
        || ui.model.borrow().anchor.is_some() && !ui.model.borrow().messages.is_empty(),
        "search result opens message context",
    );

    ui.open(id(), None);
    pump_until(
        || ui.model.borrow().selected_draft().is_some_and(|d| d.loaded),
        "thread draft reload",
    );
    // Offline account state must disable send while retaining an editable draft.
    let mut offline = fixture().0;
    offline.connected = false;
    offline.authenticated = false;
    send_json(
        subscription_writer.lock().unwrap().as_mut().unwrap(),
        response("account_updated", json!({"account":offline})),
    );
    pump_until(
        || !ui.model.borrow().accounts[0].connected,
        "account offline state event",
    );
    ui.compose.buffer().set_text("offline draft");
    assert!(!ui.send.is_sensitive());
    assert_eq!(text(&ui.compose), "offline draft");
    send_json(
        subscription_writer.lock().unwrap().as_mut().unwrap(),
        response("account_updated", json!({"account":fixture().0})),
    );
    pump_until(
        || ui.model.borrow().accounts[0].connected,
        "account online state event",
    );

    ui.compose.buffer().set_text("accepted text");
    ui.send.emit_clicked();
    pump_until(
        || sent.lock().unwrap().len() == 1 && text(&ui.compose).is_empty(),
        "accepted send clears captured draft",
    );
    assert_eq!(sent.lock().unwrap()[0], "accepted text");

    ui.compose.buffer().set_text("uncertain text");
    let previous_subscriptions = subscriptions.load(Ordering::SeqCst);
    fail_send.store(true, Ordering::SeqCst);
    ui.send.emit_clicked();
    pump_until(
        || {
            sent.lock().unwrap().len() == 2
                && ui
                    .model
                    .borrow()
                    .send_notices
                    .get(&id())
                    .is_some_and(|notice| notice.contains("outcome unknown"))
        },
        "uncertain send failure",
    );
    assert_eq!(text(&ui.compose), "uncertain text");
    let count = sent.lock().unwrap().len();
    pump_until(
        || subscriptions.load(Ordering::SeqCst) > previous_subscriptions,
        "IPC subscription reconnect after uncertain send",
    );
    assert_eq!(
        text(&ui.compose),
        "uncertain text",
        "reconnect preserves the unsent editor text"
    );
    assert_eq!(
        sent.lock().unwrap().len(),
        count,
        "reconnect must not replay an uncertain send"
    );

    // This uses the runner's fake helper on PATH. It never launches a real browser or account flow.
    ui.connect_account();
    let setup = setup_window(&app);
    pump_until(
        || {
            find_widget(setup.upcast_ref(), &|widget| {
                widget.downcast_ref::<gtk::Button>().is_some_and(|button| {
                    button.label().as_deref() == Some("Continue in browser")
                        && button.is_sensitive()
                })
            })
            .is_some()
        },
        "fake browser inventory enables setup",
    );
    let continue_button = find_widget(setup.upcast_ref(), &|widget| {
        widget
            .downcast_ref::<gtk::Button>()
            .is_some_and(|button| button.label().as_deref() == Some("Continue in browser"))
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    continue_button.emit_clicked();
    pump_until(
        || {
            find_widget(setup.upcast_ref(), &|widget| {
                widget
                    .downcast_ref::<gtk::Label>()
                    .is_some_and(|label| label.text().contains("Fake setup progress"))
            })
            .is_some()
        },
        "sanitized setup progress",
    );
    pump_until(
        || {
            find_widget(setup.upcast_ref(), &|widget| {
                widget
                    .downcast_ref::<gtk::Label>()
                    .is_some_and(|label| label.text().contains("Setup saved."))
            })
            .is_some()
        },
        "sanitized setup completion",
    );
    assert!(
        setup.is_visible(),
        "setup window remains alive through helper callbacks"
    );

    let binary = PathBuf::from(
        std::env::var_os("HANDOVER_MESSAGES_TEST_BINARY")
            .expect("runner supplies isolated native Messages binary"),
    );
    let first_thread = "first-process-thread";
    let mut first = ChildProcess(
        Command::new(&binary)
            .args(["--account", ACCOUNT, "--conversation", first_thread])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start first native Messages process"),
    );
    pump_until(
        || saw_target(&requests, "messages.select", first_thread),
        "first process routes its requested conversation",
    );
    let second_thread = "thread-2";
    let mut second = ChildProcess(
        Command::new(&binary)
            .args(["--account", ACCOUNT, "--conversation", second_thread])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start second native Messages invocation"),
    );
    pump_until(
        || second.0.try_wait().ok().flatten().is_some(),
        "second invocation exits after GApplication routing",
    );
    assert_eq!(second.0.wait().unwrap().code(), Some(0));
    pump_until(
        || saw_target(&requests, "messages.select", second_thread),
        "second invocation routes target into first process",
    );
    assert!(
        first.0.try_wait().unwrap().is_none(),
        "original GUI process remains alive after routed invocation"
    );
    assert!(saw_target(&requests, "messages.draft", second_thread));

    ui.window.destroy();
    let _ = first.0.kill();
    let _ = first.0.wait();
    drop(Rc::clone(&ui));
}
