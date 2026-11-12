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
            address: Some("+1 (312) 555-0100".into()),
            is_self: false,
        }],
        latest_message_id: None,
        last_activity_at: Some(1794407100000000),
        unread_count: Some(0),
        cursor: None,
        capabilities: [
            MessagingCapability::Text,
            MessagingCapability::Replies,
            MessagingCapability::Reactions,
            MessagingCapability::Media,
        ]
        .into_iter()
        .collect(),
    };
    (account, conversation)
}

fn make_media_fixtures(root: &std::path::Path) -> (PathBuf, String) {
    let directory = root.join("fixtures");
    std::fs::create_dir_all(&directory).expect("create private fixture directory");
    let image = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 24, 24)
        .expect("allocate fixture image");
    for y in 0..24 {
        for x in 0..24 {
            let (r, g, b) = if (x / 6 + y / 6) % 2 == 0 {
                (59, 130, 244)
            } else {
                (245, 158, 11)
            };
            image.put_pixel(x, y, r, g, b, 255);
        }
    }
    let photo = image
        .save_to_bufferv("png", &[])
        .expect("encode fixture contact PNG");
    let staged_path = directory.join("inline-map.heic");
    std::fs::write(
        &staged_path,
        include_bytes!("../../tests/fixtures/images/sample.heic"),
    )
    .expect("write fixture attachment HEIC");
    (staged_path, glib::base64_encode(&photo).to_string())
}

fn make_av_fixtures(root: &std::path::Path) -> (PathBuf, PathBuf) {
    let directory = root.join("fixtures");
    std::fs::create_dir_all(&directory).expect("create private media fixture directory");

    let audio_path = directory.join("sample.wav");
    let sample_rate = 8_000u32;
    let samples = sample_rate * 3;
    let data_len = samples * 2;
    let mut wav = Vec::with_capacity((44 + data_len) as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for index in 0..samples {
        let sample = ((index as f32 * 440.0 * std::f32::consts::TAU / sample_rate as f32).sin()
            * 400.0) as i16;
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(&audio_path, wav).expect("write short WAV fixture");

    let video_path = directory.join("sample.webm");
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=160x120:d=3",
            "-an",
            "-c:v",
            "libvpx",
            "-y",
        ])
        .arg(&video_path)
        .status()
        .expect("ffmpeg is required to create the tiny video fixture");
    assert!(
        status.success(),
        "ffmpeg failed to create the video fixture"
    );
    (audio_path, video_path)
}

fn assert_media_fd_stream_opens(path: &std::path::Path) {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let source = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .expect("open local media fixture without following symlinks");
    let fd_path = format!("/proc/self/fd/{}", source.as_raw_fd());
    assert!(
        gtk::gio::File::for_path(fd_path)
            .read(None::<&gtk::gio::Cancellable>)
            .is_ok(),
        "GTK GIO can read the pinned local media descriptor"
    );
}

fn conversations() -> Vec<Conversation> {
    let first = fixture().1;
    let mut items = vec![first];
    for (local_id, title, kind, transport, address, unread, activity) in [
        (
            "scroll-history",
            Some("Scroll history"),
            ConversationKind::Direct,
            TransportKind::Rcs,
            None,
            0,
            1794407200000000i64,
        ),
        (
            "maya-chen",
            Some("Maya Chen"),
            ConversationKind::Direct,
            TransportKind::Rcs,
            None,
            1,
            1794407000000000i64,
        ),
        (
            "phone-415",
            None,
            ConversationKind::Direct,
            TransportKind::Sms,
            Some("+1 (415) 555-0124"),
            0,
            1794406800000000i64,
        ),
        (
            "weekend-crew",
            Some("Weekend Crew"),
            ConversationKind::Group,
            TransportKind::Rcs,
            None,
            2,
            1794406600000000i64,
        ),
        (
            "priya-nair",
            Some("Priya Nair"),
            ConversationKind::Direct,
            TransportKind::Rcs,
            None,
            0,
            1794406400000000i64,
        ),
        (
            "phone-310",
            None,
            ConversationKind::Direct,
            TransportKind::Sms,
            Some("+1 (310) 555-0186"),
            0,
            1794406200000000i64,
        ),
        (
            "hiking-group",
            Some("Saturday hikers"),
            ConversationKind::Group,
            TransportKind::Mms,
            None,
            0,
            1794406000000000i64,
        ),
        (
            "jordan-lee",
            Some("Jordan Lee"),
            ConversationKind::Direct,
            TransportKind::Rcs,
            None,
            0,
            1794405800000000i64,
        ),
        (
            "family-chat",
            Some("Family"),
            ConversationKind::Group,
            TransportKind::Rcs,
            None,
            3,
            1794405600000000i64,
        ),
        (
            "mom-phone",
            Some("Mom"),
            ConversationKind::Direct,
            TransportKind::Sms,
            Some("+1 (206) 555-0142"),
            0,
            1794405400000000i64,
        ),
    ] {
        let person_name = title.map(str::to_owned);
        let address = address.map(str::to_owned);
        let participant_name = person_name.clone().or_else(|| address.clone());
        let participants = if kind == ConversationKind::Group {
            vec![
                Participant {
                    local_id: "contact-1".into(),
                    display_name: Some("Maya Chen".into()),
                    address: Some("3125550100".into()),
                    is_self: false,
                },
                Participant {
                    local_id: "contact-2".into(),
                    display_name: Some("Jordan Lee".into()),
                    address: Some("Jordan@Example.test".into()),
                    is_self: false,
                },
            ]
        } else {
            vec![Participant {
                local_id: local_id.into(),
                display_name: participant_name,
                address,
                is_self: false,
            }]
        };
        items.push(Conversation {
            id: ConversationId::new(MessagingAccountId::new(ACCOUNT), local_id),
            kind,
            transport,
            title: person_name,
            participants,
            latest_message_id: None,
            last_activity_at: Some(activity),
            unread_count: Some(unread),
            cursor: None,
            capabilities: [MessagingCapability::Text, MessagingCapability::Replies]
                .into_iter()
                .collect(),
        });
    }
    items
}

fn preview_message(conversation: &Conversation) -> Value {
    let participant = conversation.participants.first();
    let sender = Participant {
        local_id: participant
            .map_or("contact", |value| value.local_id.as_str())
            .into(),
        display_name: participant.and_then(|value| value.display_name.clone()),
        address: participant.and_then(|value| value.address.clone()),
        is_self: false,
    };
    let text = match conversation.id.local_id.as_str() {
        "thread-1" => {
            "Will do. Riverside Market looks lovely — here’s the address: https://example.test/riverside"
        }
        "maya-chen" => "I can bring coffee for everyone ☕",
        "phone-415" => "Running about ten minutes late",
        "weekend-crew" => "Maya: The north trail is open again!",
        "priya-nair" => "That sounds perfect, thank you!",
        "phone-310" => "Can you call me when you arrive?",
        "hiking-group" => "Jordan: Meet by the trailhead at 9?",
        "jordan-lee" => "Sent you the photos from yesterday",
        "family-chat" => "Dinner is at 6:30 on Sunday",
        _ => "I’m on my way now",
    };
    json!({
        "id": {"conversation_id": conversation.id, "local_id": "preview-1"},
        "sender": sender,
        "transport": conversation.transport,
        "sent_at": conversation.last_activity_at,
        "text": text,
        "attachments": [], "reply_to": null, "reactions": [], "deleted": false
    })
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
    attachment_drafts: Arc<Mutex<std::collections::BTreeMap<String, Value>>>,
}

fn fake_server(
    path: PathBuf,
    state: FixtureState,
    staged_path: PathBuf,
    photo_base64: String,
    attachment_staging: PathBuf,
) -> thread::JoinHandle<()> {
    let FixtureState {
        fail_send,
        disconnect_subscription,
        subscriptions,
        subscription_writer,
        requests,
        sent,
        drafts,
        attachment_drafts,
    } = state;
    let staged_path = staged_path.to_string_lossy().into_owned();
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
            let attachment_drafts = attachment_drafts.clone();
            let staged_path = staged_path.clone();
            let attachment_staging = attachment_staging.clone();
            let photo_base64 = photo_base64.clone();
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
                let all_conversations = conversations();
                match method {
                    "subscribe" => {
                        subscriptions.fetch_add(1, Ordering::SeqCst);
                        disconnect_subscription.store(false, Ordering::SeqCst);
                        send_json(
                            &mut stream,
                            response(
                                "subscribed",
                                json!({"devices": [], "notifications": [], "media_sessions": [], "calls": [], "messaging_accounts": [account], "conversations": all_conversations, "typing_states": [], "read_states": []}),
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
                    "messages.sync" => send_json(
                        &mut stream,
                        response(
                            "account_accepted",
                            json!({"account_id":request["account_id"]}),
                        ),
                    ),
                    "messages.workspace" | "messages.select" => send_json(
                        &mut stream,
                        response(
                            "messaging_workspace",
                            json!({"workspace": {"location": null, "draft_conversations": [], "persistent": true}}),
                        ),
                    ),
                    "messages.previews" => {
                        let requested = request["conversation_ids"]
                            .as_array()
                            .expect("preview request conversation_ids array");
                        let messages: Vec<_> = requested
                            .iter()
                            .filter_map(|value| {
                                let id: ConversationId =
                                    serde_json::from_value(value.clone()).ok()?;
                                all_conversations
                                    .iter()
                                    .find(|conversation| conversation.id == id)
                            })
                            .map(preview_message)
                            .collect();
                        send_json(
                            &mut stream,
                            response("message_previews", json!({"messages":messages,"done":true})),
                        );
                    }
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
                    "messages.attachment_draft" => {
                        let conversation_id = request["conversation_id"].clone();
                        let key = conversation_id.to_string();
                        let draft = attachment_drafts
                            .lock()
                            .unwrap()
                            .get(&key)
                            .cloned()
                            .unwrap_or_else(|| json!({"conversation_id":conversation_id,"attachment":null,"revision":0}));
                        send_json(
                            &mut stream,
                            response("message_attachment_draft", json!({"draft":draft})),
                        );
                    }
                    "messages.save_attachment_draft" => {
                        let conversation_id = request["conversation_id"].clone();
                        let key = conversation_id.to_string();
                        let mut saved = attachment_drafts.lock().unwrap();
                        let current = saved.get(&key).cloned().unwrap_or_else(|| {
                            json!({"conversation_id":conversation_id,"attachment":null,"revision":0})
                        });
                        if current["revision"].as_u64().unwrap_or(0)
                            != request["revision"].as_u64().unwrap_or(0)
                        {
                            send_json(
                                &mut stream,
                                response(
                                    "error",
                                    json!({"code":"draft_conflict","message":"Attachment draft changed"}),
                                ),
                            );
                            return;
                        }
                        let attachment = if let Some(file_url) = request["file_url"].as_str() {
                            let source = gtk::gio::File::for_uri(file_url)
                                .path()
                                .expect("fixture attachment URL is local");
                            let basename = source.file_name().expect("fixture file has a basename");
                            let directory = attachment_staging.join(format!(
                                "{}-{}",
                                request["conversation_id"]["account_id"]
                                    .as_str()
                                    .unwrap_or("account"),
                                request["conversation_id"]["local_id"]
                                    .as_str()
                                    .unwrap_or("conversation")
                            ));
                            std::fs::create_dir_all(&directory)
                                .expect("create private attachment draft staging directory");
                            let staged = directory.join(basename);
                            std::fs::copy(&source, &staged)
                                .expect("copy fixture attachment into private staging");
                            let metadata = std::fs::metadata(&staged)
                                .expect("read private staged attachment metadata");
                            json!({
                                "local_id":"fixture-attachment-draft",
                                "mime":request["mime"],
                                "name":basename.to_string_lossy(),
                                "size_bytes":metadata.len(),
                                "staged_path":staged.to_string_lossy()
                            })
                        } else {
                            Value::Null
                        };
                        let draft = json!({
                            "conversation_id":conversation_id,
                            "attachment":attachment,
                            "revision":current["revision"].as_u64().unwrap_or(0) + 1
                        });
                        saved.insert(key, draft.clone());
                        send_json(
                            &mut stream,
                            response("message_attachment_draft", json!({"draft":draft})),
                        );
                    }
                    "messages.history" => send_json(&mut stream, {
                        if request["conversation_id"]["local_id"] == "scroll-history" {
                            let older = request["cursor"].as_str() == Some("older-50");
                            if older {
                                thread::sleep(Duration::from_millis(120));
                            }
                            let start = if older { 0 } else { 50 };
                            let messages: Vec<_> = (start..start + 50).map(|index| json!({
                                    "id":{"conversation_id":request["conversation_id"],"local_id":format!("history-{index:03}")},
                                    "sender":{"local_id":"alex","display_name":"Alex Example","address":null,"is_self":false},
                                    "transport":"rcs","sent_at":1794400000000000i64 + index as i64,
                                    "text":format!("History message {index:03}\nA second line with enough text to make each row occupy visible space.\nA third line keeps the conversation taller than the viewport."),
                                    "attachments":[],"reply_to":null,"reactions":[],"deleted":false
                                })).collect();
                            response(
                                "history",
                                json!({"conversation_id":request["conversation_id"],"messages":messages,"cursor_next":if older { Value::Null } else { json!("older-50") }}),
                            )
                        } else {
                            response(
                                "history",
                                json!({"conversation_id":request["conversation_id"],"messages":[
                                {"id":{"conversation_id":conversation.id,"local_id":"m1"},"sender":{"local_id":"alex","display_name":"Alex Example","address":null,"is_self":false},"transport":"rcs","sent_at":1794406200000000i64,"text":"Hey! Are we still on for Saturday?","attachments":[],"reply_to":null,"reactions":[{"emoji":"👍","participant_ids":["alex"],"count":1}],"deleted":false},
                                {"id":{"conversation_id":conversation.id,"local_id":"m2"},"sender":{"local_id":"self","display_name":"You","address":null,"is_self":true},"transport":"rcs","sent_at":1794406500000000i64,"text":"Absolutely. I found a little place near the river that looks great. We could meet there around noon, grab lunch, and walk along the trail if the weather holds.","attachments":[],"reply_to":null,"reactions":[],"deleted":false},
                                {"id":{"conversation_id":conversation.id,"local_id":"m3"},"sender":{"local_id":"alex","display_name":"Alex Example","address":null,"is_self":false},"transport":"rcs","sent_at":1794406800000000i64,"text":"Perfect, send me the name when you get a chance.","attachments":[],"reply_to":{"conversation_id":conversation.id,"local_id":"m1"},"reactions":[],"deleted":false},
                                {"id":{"conversation_id":conversation.id,"local_id":"m4"},"sender":{"local_id":"self","display_name":"You","address":null,"is_self":true},"transport":"rcs","sent_at":1794407100000000i64,"text":"Will do. It's called Riverside Market. I'll send you the address shortly.","attachments":[],"reply_to":null,"reactions":[],"deleted":false}
                                ,{"id":{"conversation_id":conversation.id,"local_id":"m5"},"sender":{"local_id":"self","display_name":"You","address":null,"is_self":true},"transport":"rcs","sent_at":1794407160000000i64,"text":"The entrance is on Water Street, just past the old brick station. Here’s the map with the route and opening hours: https://example.test/riverside-market?day=saturday&time=12%3A00. We can meet outside if it’s busy.","attachments":[{"local_id":"map-image","mime":null,"name":"route.heic","size_bytes":null,"staged_path":staged_path},{"local_id":"market-photo","mime":"image/heic","name":"market-photo.heic","size_bytes":null,"staged_path":staged_path}],"reply_to":null,"reactions":[],"deleted":false}
                            ],"cursor_next":null}),
                            )
                        }
                    }),
                    "messages.search" => {
                        let message_id =
                            json!({"conversation_id":conversation.id,"local_id":"message-1"});
                        let hit = json!({"message_id":message_id,"snippet":"Found in saved history","sender":{"local_id":"alex","display_name":"Alex Example","address":null,"is_self":false},"sent_at":1});
                        send_json(
                            &mut stream,
                            response("message_search", json!({"hits":[hit],"has_more":false})),
                        );
                    }
                    "messages.search_conversation" => {
                        let conversation_id = request["conversation_id"].clone();
                        let offset = request["offset"].as_u64().unwrap_or(0);
                        let hits = if conversation_id["local_id"] == "thread-1"
                            && !request["query"].as_str().unwrap_or("").trim().is_empty()
                        {
                            match offset {
                                0 => vec![json!({
                                    "message_id":{"conversation_id":conversation_id,"local_id":"m1"},
                                    "snippet":"Hey! Are we still on for Saturday?",
                                    "sender":{"local_id":"alex","display_name":"Alex Example","address":null,"is_self":false},
                                    "sent_at":1794406200000000i64
                                })],
                                25 => vec![json!({
                                    "message_id":{"conversation_id":conversation_id,"local_id":"m3"},
                                    "snippet":"Perfect, send me the name when you get a chance.",
                                    "sender":{"local_id":"alex","display_name":"Alex Example","address":null,"is_self":false},
                                    "sent_at":1794406800000000i64
                                })],
                                _ => Vec::new(),
                            }
                        } else {
                            Vec::new()
                        };
                        let has_more = conversation_id["local_id"] == "thread-1"
                            && offset == 0
                            && !request["query"].as_str().unwrap_or("").trim().is_empty();
                        send_json(
                            &mut stream,
                            response("message_search", json!({"hits":hits,"has_more":has_more})),
                        );
                    }
                    "messages.context"
                        if request["message_id"]["conversation_id"]["local_id"]
                            == "scroll-history" =>
                    {
                        let id = request["message_id"]["conversation_id"].clone();
                        let messages: Vec<_> = (50..60).map(|index| json!({
                            "id":{"conversation_id":id,"local_id":format!("history-{index:03}")},
                            "sender":{"local_id":"alex","display_name":"Alex Example","address":null,"is_self":false},
                            "transport":"rcs","sent_at":1794400000000000i64 + index as i64,
                            "text":format!("History message {index:03}\nA second line with enough text to make each row occupy visible space.\nA third line keeps the conversation taller than the viewport."),
                            "attachments":[],"reply_to":null,"reactions":[],"deleted":false
                        })).collect();
                        send_json(
                            &mut stream,
                            response(
                                "message_context",
                                json!({"conversation_id":id,"messages":messages,"done":true,"cursor_next":"older-50"}),
                            ),
                        );
                    }
                    "messages.context" => {
                        let message_id = request["message_id"].clone();
                        if message_id["local_id"] == "m0" {
                            thread::sleep(Duration::from_millis(500));
                        }
                        let body = if message_id["local_id"] == "m1" {
                            "Hey! Are we still on for Saturday?"
                        } else {
                            "Found in saved history"
                        };
                        let message = json!({"id":message_id,"sender":{"local_id":"alex","display_name":"Alex Example","address":null,"is_self":false},"transport":"rcs","sent_at":1,"text":body,"attachments":[],"reply_to":null,"reactions":[],"deleted":false});
                        send_json(
                            &mut stream,
                            response(
                                "message_context",
                                json!({"conversation_id":conversation.id,"messages":[message],"done":true}),
                            ),
                        );
                    }
                    "messages.media" => send_json(
                        &mut stream,
                        response(
                            "message_media",
                            json!({
                                "conversation_id":conversation.id,
                                "messages":[{
                                    "id":{"conversation_id":conversation.id,"local_id":"m5"},
                                    "sender":{"local_id":"self","display_name":"You","address":null,"is_self":true},
                                    "transport":"rcs","sent_at":1794407160000000i64,
                                    "text":"The entrance is on Water Street.",
                                    "attachments":[
                                        {"local_id":"map-image","mime":null,"name":"route.heic","size_bytes":null,"staged_path":staged_path},
                                        {"local_id":"market-photo","mime":"image/heic","name":"market-photo.heic","size_bytes":null,"staged_path":staged_path}
                                    ],
                                    "reply_to":null,"reactions":[],"deleted":false
                                }],
                                "has_more":false,"done":true
                            }),
                        ),
                    ),
                    "messages.react" | "messages.unreact" | "messages.send_file" => send_json(
                        &mut stream,
                        response("message_accepted", json!({"request_id":"op-fixture"})),
                    ),
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
                    "contacts.list" => send_json(
                        &mut stream,
                        response(
                            "contacts",
                            json!({"contacts":[{
                                "device_id":"android:gui-test",
                                "local_id":"alex-contact",
                                "display_name":"Alex Example",
                                "phones":["+1 (312) 555-0100"],
                                "emails":[],
                                "photo":photo_base64
                            },{
                                "device_id":"android:gui-test",
                                "local_id":"jordan-contact",
                                "display_name":"Jordan Lee",
                                "phones":[],
                                "emails":["jordan@example.test"],
                                "photo":glib::base64_encode(include_bytes!("../../tests/fixtures/images/sample.png")).to_string()
                            }]}),
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

fn wait_for_layout(window: &impl IsA<gtk::Widget>) {
    let frames = Rc::new(Cell::new(0));
    let rendered = frames.clone();
    window.add_tick_callback(move |_, _| {
        rendered.set(rendered.get() + 1);
        if rendered.get() >= 3 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    pump_until(|| frames.get() >= 3, "fixture layout and paint");
}

fn capture_fixture(window: &impl IsA<gtk::Widget>, path: PathBuf) {
    wait_for_layout(window);
    let status = Command::new("import")
        .args(["-window", "root"])
        .arg(path)
        .status()
        .expect("ImageMagick import is required for requested screenshot");
    assert!(status.success(), "native fixture screenshot capture failed");
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

fn widgets_with_class(root: &gtk::Widget, class: &str) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    if root.has_css_class(class) {
        found.push(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        found.extend(widgets_with_class(&widget, class));
        child = widget.next_sibling();
    }
    found
}

fn ancestor_with_class(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    let mut current = Some(widget.clone());
    while let Some(candidate) = current {
        if candidate.has_css_class(class) {
            return Some(candidate);
        }
        current = candidate.parent();
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
    let attachment_drafts = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    gtk::init().expect("GTK display");
    adw::init().expect("libadwaita initialization");
    let (staged_path, photo_base64) = make_media_fixtures(&root);
    let (audio_path, video_path) = make_av_fixtures(&root);
    assert_media_fd_stream_opens(&audio_path);
    assert_media_fd_stream_opens(&video_path);
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
            attachment_drafts,
        },
        staged_path.clone(),
        photo_base64,
        root.join("fixture-attachment-drafts"),
    );
    assert!(
        root.join("runtime/handover/handoverd.sock").exists(),
        "private fake daemon must be bound before constructing the window"
    );
    if std::env::var_os("HANDOVER_MESSAGES_TEST_DARK").is_some() {
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    }
    let app = adw::Application::builder()
        .application_id("org.handover.Messages.GuiTest")
        .build();
    app.register(None::<&gtk::gio::Cancellable>)
        .expect("register test application");
    let ui = Window::new(&app);
    if let Ok(width) = std::env::var("HANDOVER_MESSAGES_TEST_WIDTH") {
        ui.window
            .set_default_size(width.parse().expect("fixture width"), 780);
    }
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
    pump_until(
        || ui.avatar.custom_image().is_some(),
        "synced contact photo in conversation header",
    );
    pump_until(
        || {
            find_widget(ui.list.upcast_ref(), &|widget| {
                widget.downcast_ref::<adw::Avatar>().is_some_and(|avatar| {
                    avatar.icon_name().as_deref() == Some("system-users-symbolic")
                        && avatar.custom_image().is_some()
                })
            })
            .is_some()
        },
        "group member photos in sidebar with QML-compatible address matching",
    );
    pump_until(
        || {
            find_widget(ui.window.upcast_ref(), &|widget| {
                widget
                    .downcast_ref::<crate::image_canvas::ImageCanvas>()
                    .is_some_and(|canvas| canvas.texture().is_some())
            })
            .is_some()
        },
        "decoded inline image preview",
    );
    let row_before_background_event = ui.list.selected_row();
    let thread_before_background_event = ui.thread.first_child();
    ui.event(Event::State(Box::new(ServerPayload::Media {
        media_sessions: vec![],
    })));
    assert_eq!(
        ui.list.selected_row(),
        row_before_background_event,
        "unrelated continuity events must not rebuild the entire conversation list"
    );
    assert_eq!(ui.thread.first_child(), thread_before_background_event);
    // Match a real library's scale while keeping all data synthetic.
    let original_conversations = ui.model.borrow().conversations.clone();
    let (_, prototype) = fixture();
    let mut background_conversations = Vec::new();
    for index in 0..426 {
        let mut conversation = prototype.clone();
        conversation.id.local_id = format!("background-{index}");
        conversation.title = Some(format!("Background chat {index}"));
        conversation.last_activity_at = Some(0);
        background_conversations.push(conversation);
    }
    ui.model
        .borrow_mut()
        .conversations
        .extend(background_conversations.clone());
    ui.render_sidebar();
    let row_before_burst = ui.list.selected_row();
    let thread_revision = ui.render_revision.get();
    for index in 0..100 {
        let mut conversation =
            background_conversations[index % background_conversations.len()].clone();
        conversation.title = Some(format!("Updated background chat {index}"));
        ui.event(Event::State(Box::new(ServerPayload::ConversationUpdated {
            conversation,
        })));
    }
    assert_eq!(
        ui.render_revision.get(),
        thread_revision,
        "an inventory burst for other chats must not rebuild the open thread"
    );
    assert_eq!(
        ui.list.selected_row(),
        row_before_burst,
        "inventory events coalesce before touching GTK rows"
    );
    pump_until(
        || ui.sidebar_timer.borrow().is_none(),
        "coalesced sidebar update",
    );
    assert!(
        find_widget(ui.list.upcast_ref(), &|widget| {
            widget
                .downcast_ref::<gtk::Label>()
                .is_some_and(|label| label.text() == "Updated background chat 99")
        })
        .is_some(),
        "coalescing must retain the last inventory update"
    );
    assert_eq!(ui.model.borrow().selected.as_ref(), Some(&id()));
    ui.model.borrow_mut().conversations = original_conversations;
    ui.render_sidebar();
    let image_button = find_widget(ui.thread.upcast_ref(), &|widget| {
        widget.has_css_class("image-attachment")
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    image_button.emit_clicked();
    pump_until(
        || {
            app.windows()
                .iter()
                .any(|window| window.title().as_deref() == Some("route.heic"))
        },
        "in-app image viewer",
    );
    let viewer = app
        .windows()
        .into_iter()
        .find(|window| window.title().as_deref() == Some("route.heic"))
        .unwrap();
    pump_until(
        || {
            find_widget(viewer.upcast_ref(), &|widget| {
                widget
                    .downcast_ref::<crate::image_canvas::ImageCanvas>()
                    .is_some_and(|canvas| canvas.texture().is_some())
            })
            .is_some()
        },
        "viewer image decode",
    );
    let canvas = find_widget(viewer.upcast_ref(), &|widget| {
        widget.has_css_class("image-canvas")
    })
    .unwrap()
    .downcast::<crate::image_canvas::ImageCanvas>()
    .unwrap();
    let zoom = find_widget(viewer.upcast_ref(), &|widget| {
        widget.tooltip_text().as_deref() == Some("Zoom in")
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    zoom.emit_clicked();
    assert!(canvas.zoomed_size().0 > 0);
    let fit = find_widget(viewer.upcast_ref(), &|widget| {
        widget
            .downcast_ref::<gtk::Button>()
            .is_some_and(|button| button.label().as_deref() == Some("Fit"))
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    fit.emit_clicked();
    assert_eq!(canvas.zoomed_size(), (0, 0));
    if let Some(path) = std::env::var_os("HANDOVER_MESSAGES_TEST_SCREENSHOT") {
        let path = PathBuf::from(path);
        let name = format!("{}-viewer.png", path.file_stem().unwrap().to_string_lossy());
        capture_fixture(&viewer, path.with_file_name(name));
    }
    viewer.close();

    pump_until(
        || ui.model.borrow().messages.len() >= 5,
        "fixture thread with loaded replies and reactions",
    );
    let av_message = json!({
        "id":{"conversation_id":id(),"local_id":"av-fixture"},
        "sender":{"local_id":"self","display_name":"You","address":null,"is_self":true},
        "transport":"rcs","sent_at":1794407170000000i64,
        "text":"Local playback fixtures",
        "attachments":[
            {"local_id":"audio-fixture","mime":"audio/wav","name":"sample.wav","size_bytes":null,"staged_path":audio_path},
            {"local_id":"video-fixture","mime":"video/webm","name":"sample.webm","size_bytes":null,"staged_path":video_path}
        ],
        "reply_to":null,"reactions":[],"deleted":false
    });
    send_json(
        subscription_writer.lock().unwrap().as_mut().unwrap(),
        response("message_added", json!({"message":av_message})),
    );
    pump_until(
        || {
            ui.model
                .borrow()
                .messages
                .iter()
                .any(|message| message.id.local_id == "av-fixture")
        },
        "audio and video attachment event",
    );
    for kind in ["audio-attachment", "video-attachment"] {
        let player_widget =
            find_widget(ui.thread.upcast_ref(), &|widget| widget.has_css_class(kind)).unwrap();
        assert!(
            find_widget(&player_widget, &|widget| widget
                .downcast_ref::<gtk::MediaControls>()
                .is_some()
                || widget.downcast_ref::<gtk::Video>().is_some())
            .is_none(),
            "unopened attachments must not create decoder controls"
        );
        let start = find_widget(&player_widget, &|widget| {
            widget.tooltip_text().as_deref() == Some("Load and play this attachment")
        })
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
        start.emit_clicked();
        let stream = if kind == "audio-attachment" {
            find_widget(&player_widget, &|widget| {
                widget.downcast_ref::<gtk::MediaControls>().is_some()
            })
            .unwrap()
            .downcast::<gtk::MediaControls>()
            .unwrap()
            .media_stream()
            .unwrap()
        } else {
            find_widget(&player_widget, &|widget| {
                widget.downcast_ref::<gtk::Video>().is_some()
            })
            .unwrap()
            .downcast::<gtk::Video>()
            .unwrap()
            .media_stream()
            .unwrap()
        };
        assert!(stream.is_muted(), "synthetic fixture playback stays silent");
        pump_until(
            || stream.is_prepared() || stream.error().is_some(),
            "Play prepares the generated media",
        );
        assert!(
            stream.error().is_none(),
            "fixture media must decode: {:?}",
            stream.error()
        );
        pump_until(
            || stream.timestamp() > 0 || stream.error().is_some(),
            "Play advances the media timestamp",
        );
        assert!(stream.timestamp() > 0);
        send_json(
            subscription_writer.lock().unwrap().as_mut().unwrap(),
            response(
                "message_status",
                json!({"update":{"message_id":{"conversation_id":id(),"local_id":"av-fixture"},"status":"delivered"}}),
            ),
        );
        pump_until(
            || {
                ui.model
                    .borrow()
                    .message_status(&MessageId::new(id(), "av-fixture"))
                    == Some("Delivered")
            },
            "receipt decoration during playback",
        );
        assert!(
            stream.is_playing(),
            "receipts must preserve the playing stream"
        );
        stream.pause();
        assert!(!stream.is_playing());
        stream.play();
        let row = ancestor_with_class(&player_widget, "message-row").unwrap();
        row.set_visible(false);
        pump_until(
            || {
                // MediaFile retains cached stream flags after clear; the
                // removed source establishes that the backend was unloaded.
                stream
                    .downcast_ref::<gtk::MediaFile>()
                    .is_some_and(|file| file.file().is_none())
            },
            "unmapping unloads playback",
        );
        row.set_visible(true);
    }
    send_json(
        subscription_writer.lock().unwrap().as_mut().unwrap(),
        response(
            "message_removed",
            json!({"message_id":{"conversation_id":id(),"local_id":"av-fixture"}}),
        ),
    );
    pump_until(
        || {
            ui.model
                .borrow()
                .messages
                .iter()
                .all(|message| message.id.local_id != "av-fixture")
        },
        "media fixture event removal",
    );
    let map_text = find_widget(ui.thread.upcast_ref(), &|widget| {
        widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.text().contains("The entrance is on Water Street"))
    })
    .expect("map message text label");
    let markup = map_text.downcast_ref::<gtk::Label>().unwrap().label();
    assert!(
        markup.contains(
            "href=\"https://example.test/riverside-market?day=saturday&amp;time=12%3A00\""
        ),
        "safe link markup: {markup}"
    );
    let map_row = ancestor_with_class(&map_text, "message-row").unwrap();
    let map_actions = find_widget(&map_row, &|widget| {
        widget
            .downcast_ref::<gtk::MenuButton>()
            .is_some_and(|button| button.tooltip_text().as_deref() == Some("Message actions"))
    })
    .unwrap()
    .downcast::<gtk::MenuButton>()
    .unwrap();
    map_actions.popup();
    let map_menu = map_actions.popover().expect("map message actions menu");
    pump_until(|| map_menu.is_visible(), "map message actions menu opens");
    let copy_link = find_widget(map_menu.upcast_ref(), &|widget| {
        widget
            .downcast_ref::<gtk::Button>()
            .is_some_and(|button| button.label().as_deref() == Some("Copy link"))
    })
    .expect("copy-link action");
    copy_link
        .downcast_ref::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    let copied_link = Rc::new(RefCell::new(None));
    let copied = copied_link.clone();
    ui.window
        .clipboard()
        .read_text_async(None::<&gio::Cancellable>, move |result| {
            *copied.borrow_mut() = result.ok().flatten().map(|value| value.to_string());
        });
    pump_until(
        || {
            copied_link.borrow().as_deref()
                == Some("https://example.test/riverside-market?day=saturday&time=12%3A00")
        },
        "copy-link action copies the visible HTTPS URL",
    );

    ui.find.show();
    ui.find.entry.set_text("Saturday");
    pump_until(
        || {
            requests.lock().unwrap().iter().any(|request| {
                request["method"] == "messages.search_conversation"
                    && request["conversation_id"]["local_id"] == "thread-1"
                    && request["query"] == "Saturday"
                    && request["offset"] == 0
            }) && ui.model.borrow().anchor.as_ref() == Some(&MessageId::new(id(), "m1"))
                && ui.model.borrow().messages.iter().any(|message| {
                    message.id.local_id == "m1"
                        && message
                            .text
                            .as_deref()
                            .is_some_and(|text| text.contains("Saturday"))
                })
                && !ui.restoring.get()
        },
        "conversation-scoped find selects the first match",
    );
    let highlighted = find_widget(ui.thread.upcast_ref(), &|widget| {
        widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.text().contains("Hey! Are we still on for Saturday?"))
    })
    .expect("first search result label");
    assert!(
        highlighted
            .downcast_ref::<gtk::Label>()
            .unwrap()
            .label()
            .contains("<span"),
        "search terms are rendered as a literal highlighted span"
    );
    ui.find.next.emit_clicked();
    pump_until(
        || {
            requests.lock().unwrap().iter().any(|request| {
                request["method"] == "messages.search_conversation"
                    && request["conversation_id"]["local_id"] == "thread-1"
                    && request["offset"] == 25
            }) && ui.model.borrow().anchor.as_ref() == Some(&MessageId::new(id(), "m3"))
        },
        "next find page selects the second fixture message",
    );
    ui.find.previous.emit_clicked();
    pump_until(
        || ui.model.borrow().anchor.as_ref() == Some(&MessageId::new(id(), "m1")),
        "previous find page returns to the first fixture message",
    );
    ui.find.close();
    ui.open(id(), None);
    pump_until(
        || {
            ui.model.borrow().messages.len() == 5
                && ui.model.borrow().anchor.is_none()
                && !ui.restoring.get()
        },
        "restore newest history after scoped search",
    );
    let (mut reconnect_account, _) = fixture();
    reconnect_account.connected = false;
    ui.event(Event::State(Box::new(ServerPayload::AccountUpdated {
        account: reconnect_account.clone(),
    })));
    pump_until(
        || ui.sidebar_timer.borrow().is_none(),
        "offline account sidebar update",
    );
    let reconnect_sidebar_row = ui.list.selected_row();
    gio::prelude::ActionGroupExt::activate_action(&ui.window, "reconnect", None);
    pump_until(
        || {
            requests.lock().unwrap().iter().any(|request| {
                request["method"] == "messages.sync" && request["account_id"] == ACCOUNT
            }) && ui
                .pending
                .borrow()
                .values()
                .all(|request| !matches!(request, Request::Sync(_)))
        },
        "Reconnect resumes the selected account through the existing sync contract",
    );
    assert_eq!(ui.list.selected_row(), reconnect_sidebar_row);
    assert_eq!(ui.model.borrow().notice, "Reconnect requested.");

    assert!(
        !ui.model
            .borrow()
            .accounts
            .iter()
            .find(|account| account.id.as_str() == ACCOUNT)
            .unwrap()
            .connected,
        "accepting reconnect must not invent a successful connection"
    );
    reconnect_account.connected = true;
    ui.event(Event::State(Box::new(ServerPayload::AccountUpdated {
        account: reconnect_account,
    })));
    pump_until(
        || ui.sidebar_timer.borrow().is_none(),
        "restored account sidebar update",
    );
    let selected_sidebar_row = ui.list.selected_row();
    send_json(
        subscription_writer.lock().unwrap().as_mut().unwrap(),
        serde_json::to_value(handover_ipc::ServerMessage::from_messaging_event(
            MessagingEvent::Typing(TypingState {
                conversation_id: id(),
                participant_ids: vec!["alex".into()],
            }),
        ))
        .unwrap(),
    );
    pump_until(
        || {
            find_widget(ui.window.upcast_ref(), &|widget| {
                widget.downcast_ref::<gtk::Label>().is_some_and(|label| {
                    label.has_css_class("typing-indicator")
                        && label.is_visible()
                        && label.text() == "Typing…"
                        && label.tooltip_text().as_deref() == Some("Alex Example is typing…")
                })
            })
            .is_some()
        },
        "typing indicator for selected conversation",
    );
    assert!(
        ancestor_with_class(ui.typing_indicator.upcast_ref(), "contact-heading").is_some(),
        "typing must be visible in the conversation header, as in QML"
    );
    assert!(!ui.subtitle.is_visible());
    assert_eq!(ui.composer_typing.text(), "Alex Example is typing…");
    assert!(ancestor_with_class(ui.composer_typing.upcast_ref(), "composer-area").is_some());
    assert_eq!(ui.list.selected_row(), selected_sidebar_row);
    let sidebar_typing = ui
        .sidebar_previews
        .borrow()
        .iter()
        .find(|preview| preview.conversation == id())
        .unwrap()
        .label
        .upgrade()
        .unwrap();
    assert_eq!(sidebar_typing.text(), ui.typing_indicator.text());
    assert!(sidebar_typing.has_css_class("typing-preview"));
    wait_for_layout(&ui.window);
    assert!(ui.typing_indicator.is_mapped());
    assert!(ui.composer_typing.is_mapped());
    let typing_bounds = ui.composer_typing.compute_bounds(&ui.window).unwrap();
    let compose_bounds = ui.compose.compute_bounds(&ui.window).unwrap();
    assert!(
        typing_bounds.y() + typing_bounds.height() <= compose_bounds.y(),
        "participant typing appears directly above the message box"
    );
    assert!(
        !ui.typing_indicator.layout().is_ellipsized(),
        "Typing stays readable in narrow windows"
    );
    let bounds = ui.typing_indicator.compute_bounds(&ui.window).unwrap();
    assert!(
        bounds.y() >= 0.0 && bounds.y() < 90.0 && bounds.height() > 0.0,
        "typing is placed at the top of the window"
    );
    if let Some(path) = std::env::var_os("HANDOVER_MESSAGES_TEST_SCREENSHOT") {
        let path = PathBuf::from(path);
        let name = format!("{}-typing.png", path.file_stem().unwrap().to_string_lossy());
        capture_fixture(&ui.window, path.with_file_name(name));
    }
    send_json(
        subscription_writer.lock().unwrap().as_mut().unwrap(),
        serde_json::to_value(handover_ipc::ServerMessage::from_messaging_event(
            MessagingEvent::Typing(TypingState {
                conversation_id: id(),
                participant_ids: vec![],
            }),
        ))
        .unwrap(),
    );
    pump_until(
        || {
            find_widget(ui.window.upcast_ref(), &|widget| {
                widget.downcast_ref::<gtk::Label>().is_some_and(|label| {
                    label.has_css_class("typing-indicator") && label.text().is_empty()
                })
            })
            .is_some()
        },
        "typing indicator clears after empty participant update",
    );
    assert!(!ui.typing_indicator.is_visible());
    assert!(ui.composer_typing.text().is_empty());
    assert!(ui.subtitle.is_visible());
    assert!(!sidebar_typing.has_css_class("typing-preview"));
    assert_eq!(ui.list.selected_row(), selected_sidebar_row);
    let other_thread = ConversationId::new(MessagingAccountId::new(ACCOUNT), "maya-chen");
    send_json(
        subscription_writer.lock().unwrap().as_mut().unwrap(),
        response(
            "typing",
            json!({"state":{"conversation_id":other_thread,"participant_ids":["maya-chen"]}}),
        ),
    );
    pump_until(
        || {
            ui.sidebar_previews.borrow().iter().any(|preview| {
                preview.conversation == other_thread
                    && preview.label.upgrade().is_some_and(|label| {
                        label.text().as_str() == "Typing…"
                            && label.tooltip_text().as_deref() == Some("Maya Chen is typing…")
                    })
            })
        },
        "typing in another conversation appears in its sidebar preview",
    );
    assert!(
        !ui.typing_indicator.is_visible(),
        "another chat cannot change this header"
    );
    assert_eq!(ui.model.borrow().selected.as_ref(), Some(&id()));
    send_json(
        subscription_writer.lock().unwrap().as_mut().unwrap(),
        response(
            "typing",
            json!({"state":{"conversation_id":other_thread,"participant_ids":[]}}),
        ),
    );
    pump_until(
        || {
            ui.sidebar_previews
                .borrow()
                .iter()
                .filter(|preview| preview.conversation == other_thread)
                .all(|preview| {
                    preview
                        .label
                        .upgrade()
                        .is_some_and(|label| !label.has_css_class("typing-preview"))
                })
        },
        "other chat restores its message preview when typing stops",
    );

    let m2 = MessageId::new(id(), "m2");
    for status in ["delivered", "displayed"] {
        send_json(
            subscription_writer.lock().unwrap().as_mut().unwrap(),
            response(
                "message_status",
                json!({"update":{"message_id":m2,"status":status}}),
            ),
        );
        let expected = if status == "delivered" {
            "Delivered"
        } else {
            "Read"
        };
        pump_until(
            || {
                let statuses = widgets_with_class(ui.thread.upcast_ref(), "message-status");
                statuses.len() == 1
                    && statuses[0]
                        .downcast_ref::<gtk::Label>()
                        .is_some_and(|label| label.text() == expected)
            },
            "status for the single updated outgoing message",
        );
    }

    let quoted = find_widget(ui.thread.upcast_ref(), &|widget| {
        widget.has_css_class("quoted-message")
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    let quote_label = quoted.label().unwrap_or_default();
    assert!(
        quote_label.contains("Alex Example"),
        "quote sender: {quote_label}"
    );
    assert!(
        quote_label.contains("Hey! Are we still on for Saturday?"),
        "quote text: {quote_label}"
    );
    let context_requests_before = requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request["method"] == "messages.context")
        .count();
    quoted.emit_clicked();
    pump_until(
        || {
            find_widget(ui.thread.upcast_ref(), &|widget| {
                widget.has_css_class("quote-target")
            })
            .is_some()
        },
        "quoted message jumps to loaded original",
    );
    assert_eq!(
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request["method"] == "messages.context")
            .count(),
        context_requests_before,
        "jumping to an already loaded quote does not fetch message context"
    );

    let missing_original_reply = json!({
        "id":{"conversation_id":id(),"local_id":"m6"},
        "sender":{"local_id":"alex","display_name":"Alex Example","address":null,"is_self":false},
        "transport":"rcs","sent_at":1794407180000000i64,
        "text":"A final note from the earlier conversation.",
        "attachments":[],
        "reply_to":{"conversation_id":id(),"local_id":"m0"},
        "reactions":[],"deleted":false
    });
    send_json(
        subscription_writer.lock().unwrap().as_mut().unwrap(),
        response(
            "message_added",
            json!({"message":missing_original_reply.clone()}),
        ),
    );
    pump_until(
        || ui.model.borrow().messages.len() == 6,
        "subscription adds a reply to an unloaded original",
    );
    let messages_before_missing_quote = ui.model.borrow().messages.clone();
    let _missing_quote = find_widget(ui.thread.upcast_ref(), &|widget| {
        widget.downcast_ref::<gtk::Button>().is_some_and(|button| {
            button.has_css_class("quoted-message")
                && button
                    .label()
                    .is_some_and(|label| label.contains("Original message unavailable locally"))
        })
    })
    .expect("reply to an unloaded original starts with a fallback quote");
    pump_until(
        || {
            find_widget(ui.thread.upcast_ref(), &|widget| {
                widget.downcast_ref::<gtk::Button>().is_some_and(|button| {
                    button.has_css_class("quoted-message")
                        && button
                            .label()
                            .is_some_and(|label| label.contains("Found in saved history"))
                })
            })
            .is_some()
        },
        "unloaded quote resolves from saved message context",
    );
    assert_eq!(
        ui.model.borrow().messages,
        messages_before_missing_quote,
        "fetching quote context does not replace or change the current thread"
    );
    let resolved_quote = find_widget(ui.thread.upcast_ref(), &|widget| {
        widget.downcast_ref::<gtk::Button>().is_some_and(|button| {
            button.has_css_class("quoted-message")
                && button
                    .label()
                    .is_some_and(|label| label.contains("Found in saved history"))
        })
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    resolved_quote.emit_clicked();
    let missing_target = MessageId::new(id(), "m0");
    pump_until(
        || {
            let model = ui.model.borrow();
            model.anchor.as_ref() == Some(&missing_target)
                && model.messages.iter().any(|message| {
                    message.id == missing_target
                        && message.text.as_deref() == Some("Found in saved history")
                })
        },
        "resolved quote opens the original saved message context",
    );
    ui.open(id(), None);
    pump_until(
        || {
            let model = ui.model.borrow();
            model.selected.as_ref() == Some(&id())
                && model.anchor.is_none()
                && model.messages.len() >= 5
                && model
                    .messages
                    .last()
                    .is_some_and(|message| message.id.local_id == "m5")
        },
        "returning from quoted context reopens latest messages",
    );
    send_json(
        subscription_writer.lock().unwrap().as_mut().unwrap(),
        response("message_added", json!({"message":missing_original_reply})),
    );
    pump_until(
        || {
            find_widget(ui.thread.upcast_ref(), &|widget| {
                widget.downcast_ref::<gtk::Button>().is_some_and(|button| {
                    button.has_css_class("quoted-message")
                        && button
                            .label()
                            .is_some_and(|label| label.contains("Found in saved history"))
                })
            })
            .is_some()
        },
        "latest thread refetches its unloaded quote",
    );
    let updated_original = json!({
        "id":{"conversation_id":id(),"local_id":"m0"},
        "sender":{"local_id":"alex","display_name":"Alex Example","address":null,"is_self":false},
        "transport":"rcs","sent_at":1,
        "text":"Updated saved history text",
        "attachments":[],"reply_to":null,"reactions":[],"deleted":false
    });
    send_json(
        subscription_writer.lock().unwrap().as_mut().unwrap(),
        response("message_updated", json!({"message":updated_original})),
    );
    pump_until(
        || {
            find_widget(ui.thread.upcast_ref(), &|widget| {
                widget.downcast_ref::<gtk::Button>().is_some_and(|button| {
                    button.has_css_class("quoted-message")
                        && button
                            .label()
                            .is_some_and(|label| label.contains("Updated saved history text"))
                })
            })
            .is_some()
        },
        "subscription update refreshes a cached quote",
    );
    send_json(
        subscription_writer.lock().unwrap().as_mut().unwrap(),
        response(
            "message_removed",
            json!({"message_id":{"conversation_id":id(),"local_id":"m0"}}),
        ),
    );
    pump_until(
        || {
            find_widget(ui.thread.upcast_ref(), &|widget| {
                widget.downcast_ref::<gtk::Button>().is_some_and(|button| {
                    button.has_css_class("quoted-message")
                        && button.label().is_some_and(|label| {
                            label.contains("Original message unavailable locally")
                        })
                })
            })
            .is_some()
                && ui
                    .model
                    .borrow()
                    .messages
                    .iter()
                    .any(|message| message.id.local_id == "m6")
        },
        "subscription removal invalidates a cached quote",
    );

    let original_text = find_widget(ui.thread.upcast_ref(), &|widget| {
        widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.text() == "Hey! Are we still on for Saturday?")
    })
    .unwrap();
    let message_row = ancestor_with_class(&original_text, "message-row").unwrap();
    // At narrow widths the older row can be outside the scroll viewport.
    // Bring it into view as a user would before opening its actions.
    ui.jump_to(&MessageId {
        conversation_id: id(),
        local_id: "m1".into(),
    });
    wait_for_layout(&ui.window);
    let actions = find_widget(&message_row, &|widget| {
        widget
            .downcast_ref::<gtk::MenuButton>()
            .is_some_and(|button| button.tooltip_text().as_deref() == Some("Message actions"))
    })
    .unwrap()
    .downcast::<gtk::MenuButton>()
    .unwrap();
    actions.popup();
    let actions_popover = actions.popover().expect("message actions popover");
    pump_until(
        || actions_popover.is_visible(),
        "message actions menu opens",
    );
    let copy_action = find_widget(actions_popover.upcast_ref(), &|widget| {
        widget
            .downcast_ref::<gtk::Button>()
            .is_some_and(|button| button.label().as_deref() == Some("Copy text"))
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    copy_action.emit_clicked();
    let copied_text = Rc::new(RefCell::new(None));
    let copied = copied_text.clone();
    ui.window
        .clipboard()
        .read_text_async(None::<&gio::Cancellable>, move |result| {
            *copied.borrow_mut() = result.ok().flatten().map(|value| value.to_string());
        });
    pump_until(
        || copied_text.borrow().as_deref() == Some("Hey! Are we still on for Saturday?"),
        "message actions copies original text",
    );
    pump_until(
        || !actions_popover.is_visible() && !actions.is_active(),
        "copy action finishes closing its menu before reopening",
    );
    actions.popup();
    pump_until(
        || actions_popover.is_visible(),
        "message actions menu reopens",
    );
    let reply_action = find_widget(actions_popover.upcast_ref(), &|widget| {
        widget
            .downcast_ref::<gtk::Button>()
            .is_some_and(|button| button.label().as_deref() == Some("Reply"))
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    reply_action.emit_clicked();
    pump_until(
        || {
            ui.model
                .borrow()
                .selected_draft()
                .is_some_and(|draft| draft.value.reply_to == Some(MessageId::new(id(), "m1")))
        },
        "message actions reply targets original message",
    );
    ui.set_reply(None);

    let reaction_before = find_widget(ui.thread.upcast_ref(), &|widget| {
        widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.text() == "👍 1")
    })
    .expect("initial reaction aggregate");
    actions.popup();
    let reaction_action = find_widget(actions_popover.upcast_ref(), &|widget| {
        widget
            .downcast_ref::<gtk::Button>()
            .is_some_and(|button| button.label().as_deref() == Some("👍"))
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    reaction_action.emit_clicked();
    pump_until(
        || {
            requests.lock().unwrap().iter().any(|request| {
                request["method"] == "messages.react"
                    && request["message_id"]["local_id"] == "m1"
                    && request["emoji"] == "👍"
            })
        },
        "reaction request uses normalized message id",
    );
    assert_eq!(
        reaction_before.downcast_ref::<gtk::Label>().unwrap().text(),
        "👍 1",
        "accepted reaction does not optimistically change the aggregate"
    );
    actions.popup();
    let remove_reaction = find_widget(actions_popover.upcast_ref(), &|widget| {
        widget
            .downcast_ref::<gtk::Button>()
            .is_some_and(|button| button.label().as_deref() == Some("Remove my 👍 reaction"))
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    remove_reaction.emit_clicked();
    pump_until(
        || {
            requests.lock().unwrap().iter().any(|request| {
                request["method"] == "messages.unreact"
                    && request["message_id"]["local_id"] == "m1"
                    && request["emoji"] == "👍"
            })
        },
        "reaction removal request uses normalized message id",
    );
    assert_eq!(
        reaction_before.downcast_ref::<gtk::Label>().unwrap().text(),
        "👍 1",
        "accepted removal does not optimistically change the aggregate"
    );

    ui.media_button.emit_clicked();
    pump_until(
        || {
            app.windows()
                .iter()
                .any(|window| window.title().as_deref() == Some("Photos and files"))
        },
        "media browser opens from conversation header",
    );
    pump_until(
        || {
            requests
                .lock()
                .unwrap()
                .iter()
                .any(|request| request["method"] == "messages.media")
        },
        "media browser requests saved media",
    );
    let browser = app
        .windows()
        .into_iter()
        .find(|window| window.title().as_deref() == Some("Photos and files"))
        .unwrap();
    let first_photo = find_widget(browser.upcast_ref(), &|widget| {
        widget.has_css_class("image-attachment")
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    first_photo.emit_clicked();
    pump_until(
        || {
            app.windows()
                .iter()
                .any(|window| window.title().as_deref() == Some("route.heic"))
        },
        "first media gallery image opens",
    );
    let gallery_viewer = app
        .windows()
        .into_iter()
        .find(|window| window.title().as_deref() == Some("route.heic"))
        .unwrap();
    let next_photo = find_widget(gallery_viewer.upcast_ref(), &|widget| {
        widget.tooltip_text().as_deref() == Some("Next image")
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    next_photo.emit_clicked();
    pump_until(
        || {
            app.windows()
                .iter()
                .any(|window| window.title().as_deref() == Some("market-photo.heic"))
        },
        "gallery navigation changes the displayed filename",
    );
    gallery_viewer.close();
    browser.close();

    let staged_file = gio::File::for_path(&staged_path);
    ui.compose.buffer().set_text("caption to keep");
    pump_until(
        || {
            ui.model
                .borrow()
                .selected_draft()
                .is_some_and(|draft| !draft.dirty && draft.value.text == "caption to keep")
        },
        "attachment caption saves before drop",
    );
    let controllers = ui.stack.observe_controllers();
    let drop_target = (0..controllers.n_items())
        .find_map(|index| {
            controllers
                .item(index)
                .and_then(|object| object.downcast::<gtk::DropTarget>().ok())
        })
        .expect("conversation stack file drop controller");
    let one_file = gtk::gdk::FileList::from_array(std::slice::from_ref(&staged_file));
    let one_file_value = gtk::glib::BoxedValue(one_file.to_value()).to_value();
    assert!(
        drop_target.emit_by_name::<bool>("drop", &[&one_file_value, &0.0f64, &0.0f64]),
        "single local file drop is accepted"
    );
    pump_until(
        || ui.attachment_row.is_visible(),
        "staged attachment preview appears",
    );
    assert!(
        !requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request["method"] == "messages.send_file"),
        "staging an attachment does not send it"
    );
    pump_until(
        || {
            ui.attachment_state
                .borrow()
                .get(&id())
                .is_some_and(|draft| draft.attachment.is_some() && !ui.unsaved_attachment(&id()))
        },
        "attachment draft is acknowledged as saved",
    );
    let attachment_state = ui.attachment_state.borrow();
    let saved_path = attachment_state[&id()]
        .attachment
        .as_ref()
        .and_then(|attachment| attachment.staged_path.as_deref())
        .expect("saved attachment has a private staged path");
    assert_ne!(saved_path, staged_path.to_string_lossy());
    assert!(PathBuf::from(saved_path).starts_with(root.join("fixture-attachment-drafts")));
    assert_eq!(
        PathBuf::from(saved_path).file_name(),
        staged_path.file_name(),
        "fake daemon copy keeps the fixture basename"
    );
    drop(attachment_state);
    let multiple_files =
        gtk::gdk::FileList::from_array(&[staged_file.clone(), staged_file.clone()]);
    let multiple_files_value = gtk::glib::BoxedValue(multiple_files.to_value()).to_value();
    assert!(
        !drop_target.emit_by_name::<bool>("drop", &[&multiple_files_value, &0.0f64, &0.0f64]),
        "multiple file drop is rejected"
    );
    assert!(
        ui.attachment_row.is_visible(),
        "rejected drop preserves staged file"
    );
    assert_eq!(
        text(&ui.compose),
        "caption to keep",
        "rejected drop preserves existing caption"
    );
    let remove_attachment = find_widget(ui.attachment_row.upcast_ref(), &|widget| {
        widget
            .downcast_ref::<gtk::Button>()
            .is_some_and(|button| button.tooltip_text().as_deref() == Some("Remove attachment"))
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    remove_attachment.emit_clicked();
    pump_until(
        || {
            !ui.attachment_row.is_visible()
                && ui
                    .attachment_state
                    .borrow()
                    .get(&id())
                    .is_some_and(|draft| draft.attachment.is_none() && draft.revision > 0)
        },
        "attachment can be removed from the draft",
    );
    assert_eq!(
        text(&ui.compose),
        "caption to keep",
        "removing media retains its caption"
    );
    assert!(
        ui.stage_file(id(), &staged_file),
        "fixture image can be staged again"
    );
    pump_until(
        || {
            ui.attachment_state
                .borrow()
                .get(&id())
                .is_some_and(|draft| draft.attachment.is_some() && !ui.unsaved_attachment(&id()))
        },
        "restaged attachment save is acknowledged before sending",
    );
    let first_subscription = subscription_writer
        .lock()
        .unwrap()
        .as_ref()
        .expect("first window subscription remains connected")
        .try_clone()
        .unwrap();
    std::fs::remove_file(&staged_path).expect("original fixture can be removed after staging");
    let restored_window = Window::new(&app);
    restored_window.window.present();
    pump_until(
        || restored_window.model.borrow().connected,
        "second window connects to the same fake daemon",
    );
    restored_window.open(id(), None);
    pump_until(
        || {
            restored_window.model.borrow().selected.as_ref() == Some(&id())
                && restored_window
                    .attachment_state
                    .borrow()
                    .get(&id())
                    .and_then(|draft| draft.attachment.as_ref())
                    .and_then(|attachment| attachment.staged_path.as_ref())
                    .is_some_and(|path| {
                        PathBuf::from(path).starts_with(root.join("fixture-attachment-drafts"))
                            && PathBuf::from(path).is_file()
                    })
        },
        "new window restores private attachment after original source is removed",
    );
    restored_window.window.close();
    drop(restored_window);
    pump_until(
        || subscription_writer.lock().unwrap().is_none(),
        "second window subscription closes",
    );
    *subscription_writer.lock().unwrap() = Some(first_subscription);

    let other = ConversationId::new(MessagingAccountId::new(ACCOUNT), "other");
    ui.open(other.clone(), None);
    pump_until(
        || ui.model.borrow().selected.as_ref() == Some(&other),
        "switch away from attachment draft",
    );
    ui.open(id(), None);
    pump_until(
        || {
            ui.model.borrow().selected.as_ref() == Some(&id())
                && ui
                    .attachment_state
                    .borrow()
                    .get(&id())
                    .is_some_and(|draft| {
                        draft.attachment.as_ref().is_some_and(|attachment| {
                            attachment.staged_path.as_deref().is_some_and(|path| {
                                PathBuf::from(path)
                                    .starts_with(root.join("fixture-attachment-drafts"))
                            })
                        })
                    })
                && ui.attachment_row.is_visible()
                && ui.send.is_sensitive()
        },
        "saved attachment draft restores after switching conversations",
    );
    ui.send.emit_clicked();
    pump_until(
        || {
            !ui.attachment_row.is_visible()
                && text(&ui.compose).is_empty()
                && requests.lock().unwrap().iter().any(|request| {
                    request["method"] == "messages.send_file"
                        && request["conversation_id"]["local_id"] == "thread-1"
                        && request["caption"] == "caption to keep"
                })
                && ui
                    .attachment_state
                    .borrow()
                    .get(&id())
                    .is_some_and(|draft| draft.attachment.is_none() && draft.revision > 0)
        },
        "accepted attachment send retains caption in request then clears captured draft",
    );
    ui.compose.buffer().set_text("");
    pump_until(
        || {
            ui.model
                .borrow()
                .selected_draft()
                .is_some_and(|draft| !draft.dirty)
        },
        "empty draft saved before existing round-trip checks",
    );

    if let Some(path) = std::env::var_os("HANDOVER_MESSAGES_TEST_SCREENSHOT") {
        pump_until(
            || {
                ui.model.borrow().messages.len() >= 4
                    && ui.model.borrow().previews.len() >= conversations().len()
                    && requests
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|request| request["method"] == "messages.previews")
            },
            "history and daemon-requested conversation previews for screenshot",
        );
        capture_fixture(&ui.window, PathBuf::from(path));
    }

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

    send_json(
        subscription_writer.lock().unwrap().as_mut().unwrap(),
        response(
            "typing",
            json!({"state":{"conversation_id":id(),"participant_ids":["alex"]}}),
        ),
    );
    pump_until(
        || ui.typing_indicator.text() == "Typing…",
        "typing before interrupted send",
    );
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
    pump_until(
        || ui.typing_indicator.text().is_empty(),
        "reconnect clears stale typing",
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
    if let Some(path) = std::env::var_os("HANDOVER_MESSAGES_TEST_SCREENSHOT") {
        let path = PathBuf::from(path);
        let name = format!("{}-setup.png", path.file_stem().unwrap().to_string_lossy());
        capture_fixture(&setup, path.with_file_name(name));
    }
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

    // Keep this window's subscription separate from the routing subprocess below.
    let mut ui_subscription = subscription_writer
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .try_clone()
        .unwrap();
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

    let history_id = ConversationId::new(MessagingAccountId::new(ACCOUNT), "scroll-history");
    ui.open(history_id.clone(), None);
    pump_until(
        || {
            ui.model.borrow().selected.as_ref() == Some(&history_id)
                && ui.model.borrow().messages.len() == 50
                && !ui.restoring.get()
                && !ui.loading_latest.get()
                && ui.scroll.vadjustment().upper() > ui.scroll.vadjustment().page_size()
        },
        "overflowing newest page in synthetic history",
    );
    let initial_requests = requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| {
            request["method"] == "messages.history"
                && request["conversation_id"]["local_id"] == "scroll-history"
        })
        .count();
    assert_eq!(initial_requests, 1);
    let retained_row = ui.thread.first_child().unwrap().next_sibling().unwrap();
    let retained_id = ui.rendered.borrow()[1].clone();
    let anchor = ui.rendered.borrow()[0].clone();
    let anchor_offset = message_bounds(&ui.thread.first_child().unwrap(), &ui.thread).0;
    ui.scroll.vadjustment().set_value(0.0);
    pump_until(
        || {
            requests.lock().unwrap().iter().any(|request| {
                request["method"] == "messages.history"
                    && request["conversation_id"]["local_id"] == "scroll-history"
                    && request["cursor"] == "older-50"
            })
        },
        "automatic older-page request at scroll top",
    );
    for _ in 0..5 {
        ui.scroll.vadjustment().set_value(1.0);
        ui.scroll.vadjustment().set_value(0.0);
    }
    pump_until(
        || ui.model.borrow().messages.len() == 100 && ui.model.borrow().cursor.is_none(),
        "older page prepended and history exhausted",
    );
    pump_until(|| !ui.restoring.get(), "viewport restored after prepend");
    let index = ui
        .rendered
        .borrow()
        .iter()
        .position(|id| id == &anchor)
        .unwrap();
    let mut widget = ui.thread.first_child().unwrap();
    for _ in 0..index {
        widget = widget.next_sibling().unwrap();
    }
    let restored_offset = message_bounds(&widget, &ui.thread).0 - ui.scroll.vadjustment().value();
    let index = ui
        .rendered
        .borrow()
        .iter()
        .position(|id| id == &retained_id)
        .unwrap();
    let mut row = ui.thread.first_child().unwrap();
    for _ in 0..index {
        row = row.next_sibling().unwrap();
    }
    assert_eq!(
        row, retained_row,
        "unchanged message rows should survive prepending history"
    );
    assert!(
        (restored_offset - anchor_offset).abs() <= 2.0,
        "prepending moved the visible message: {anchor_offset} -> {restored_offset}"
    );

    let incoming = json!({
        "id":{"conversation_id":history_id,"local_id":"live-arrival"},
        "sender":{"local_id":"alex","display_name":"Alex Example","address":null,"is_self":false},
        "transport":"rcs","sent_at":1794408000000000i64,
        "text":"A new message arrived while browsing history.",
        "attachments":[],"reply_to":null,"reactions":[],"deleted":false
    });
    send_json(
        &mut ui_subscription,
        response("message_added", json!({"message":incoming})),
    );
    pump_until(
        || ui.latest.label().as_deref() == Some("1 new message ↓"),
        "one new incoming message while browsing history",
    );
    let anchor_index = ui
        .rendered
        .borrow()
        .iter()
        .position(|message_id| message_id == &anchor)
        .expect("visible history anchor remains rendered");
    let mut anchor_widget = ui.thread.first_child().unwrap();
    for _ in 0..anchor_index {
        anchor_widget = anchor_widget.next_sibling().unwrap();
    }
    let arrival_anchor_offset =
        message_bounds(&anchor_widget, &ui.thread).0 - ui.scroll.vadjustment().value();
    assert!(
        (arrival_anchor_offset - anchor_offset).abs() <= 2.0,
        "incoming message moved the visible history anchor: {anchor_offset} -> {arrival_anchor_offset}"
    );
    let incoming_update = json!({
        "id":{"conversation_id":history_id,"local_id":"live-arrival"},
        "sender":{"local_id":"alex","display_name":"Alex Example","address":null,"is_self":false},
        "transport":"rcs","sent_at":1794408000000000i64,
        "text":"The same incoming message was updated.",
        "attachments":[],"reply_to":null,"reactions":[],"deleted":false
    });
    send_json(
        &mut ui_subscription,
        response("message_updated", json!({"message":incoming_update})),
    );
    pump_until(
        || {
            ui.model
                .borrow()
                .messages
                .iter()
                .any(|message| message.id.local_id == "live-arrival")
        },
        "duplicate message update applied",
    );
    assert_eq!(ui.latest.label().as_deref(), Some("1 new message ↓"));
    ui.latest.emit_clicked();
    pump_until(
        || {
            ui.model.borrow().new_messages.is_empty()
                && ui.scroll.vadjustment().value() + ui.scroll.vadjustment().page_size()
                    >= ui.scroll.vadjustment().upper() - 2.0
        },
        "latest action clears new-message count and scrolls to end",
    );

    let history_requests = || {
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| {
                request["method"] == "messages.history"
                    && request["conversation_id"]["local_id"] == "scroll-history"
            })
            .count()
    };
    assert_eq!(
        history_requests(),
        2,
        "only the initial and one older-page request are expected"
    );
    ui.scroll.vadjustment().set_value(0.0);
    pump_until(
        || ui.scroll.vadjustment().value() == 0.0,
        "repeated top scroll settles",
    );
    thread::sleep(Duration::from_millis(250));
    assert_eq!(
        history_requests(),
        2,
        "exhausted history must not issue more requests"
    );

    // Returning to a retained tail is a scroll, not another history fetch.
    ui.latest.emit_clicked();
    pump_until(
        || {
            !ui.loading_latest.get()
                && ui.scroll.vadjustment().value() + ui.scroll.vadjustment().page_size()
                    >= ui.scroll.vadjustment().upper() - 2.0
        },
        "jump to retained latest messages",
    );
    assert_eq!(history_requests(), 2);
    // A restored location carries its real continuation and pages normally.
    let restored = MessageId::new(history_id.clone(), "history-055");
    ui.open(history_id.clone(), Some(restored.clone()));
    pump_until(
        || !ui.restoring.get() && ui.model.borrow().messages.len() >= 10,
        "restored message context",
    );
    ui.scroll.vadjustment().set_value(0.0);
    pump_until(
        || {
            !ui.restoring.get()
                && ui.model.borrow().messages.len() == 60
                && ui.model.borrow().cursor.is_none()
        },
        "automatic pagination from restored context",
    );
    assert_eq!(ui.model.borrow().anchor.as_ref(), Some(&restored));
    // An old older-page reply must not contaminate a fresh latest view of the same chat.
    ui.open(history_id.clone(), None);
    pump_until(
        || {
            ui.model.borrow().messages.len() == 50
                && !ui.restoring.get()
                && !ui.loading_latest.get()
        },
        "latest page before reopen race",
    );
    ui.scroll.vadjustment().set_value(0.0);
    pump_until(
        || {
            ui.pending.borrow().values().any(
                |request| matches!(request, Request::History(id, Some(_)) if id == &history_id),
            )
        },
        "older request in flight",
    );
    ui.open(history_id.clone(), None);
    pump_until(
        || {
            !ui.restoring.get()
                && !ui.loading_latest.get()
                && !ui
                    .pending
                    .borrow()
                    .values()
                    .any(|request| matches!(request, Request::History(..)))
        },
        "fresh latest view after delayed old reply",
    );
    assert_eq!(ui.model.borrow().messages.len(), 50);
    assert_eq!(ui.model.borrow().cursor.as_deref(), Some("older-50"));

    ui.window.destroy();
    let _ = first.0.kill();
    let _ = first.0.wait();
    drop(Rc::clone(&ui));
}
