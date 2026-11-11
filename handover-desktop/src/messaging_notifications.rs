use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    process::Stdio,
    time::{SystemTime, UNIX_EPOCH},
};

use futures_util::StreamExt;
use handover_core::{Conversation, MessageId, MessagingAccount};
use handover_ipc::{Client, ServerPayload};
use tokio::{
    process::Command,
    sync::{mpsc, watch},
    time::timeout,
};
use tracing::warn;
use zbus::{Connection, Proxy, fdo::DBusProxy, zvariant::Value};

const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
const SEEN: usize = 512;
const VISIBLE: usize = 128;

pub async fn run_messages(
    path: PathBuf,
    connection: Connection,
    mut shutdown: watch::Receiver<bool>,
) {
    let launcher = launcher_path();
    while !*shutdown.borrow() {
        if session(&path, &launcher, &connection, &mut shutdown)
            .await
            .is_err()
        {
            warn!("message notification session ended; reconnecting");
        }
        if *shutdown.borrow() {
            break;
        }
        tokio::select! { _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}, changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow() { break; } }
    }
}

struct Reader(tokio::task::JoinHandle<()>);
impl Drop for Reader {
    fn drop(&mut self) {
        self.0.abort();
    }
}

enum Update {
    Snapshot(Vec<MessagingAccount>, Vec<Conversation>),
    Payload(Box<ServerPayload>),
}

fn launcher_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("handover-messages")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("handover-messages"))
}

async fn session(
    path: &Path,
    launcher: &Path,
    connection: &Connection,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let bus = DBusProxy::new(connection).await?;
    let mut owners = bus
        .receive_name_owner_changed_with_args(&[(0, SERVICE)])
        .await?;
    let owner = loop {
        match timeout(TIMEOUT, bus.get_name_owner(SERVICE.try_into()?)).await? {
            Ok(owner) => break owner,
            Err(zbus::fdo::Error::NameHasNoOwner(_)) => {
                tokio::select! { change = owners.next() => { change.ok_or("session bus disconnected")?; }, changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow() { return Ok(()); } }
            }
            Err(error) => return Err(error.into()),
        }
    };
    let proxy = Proxy::new(connection, owner, PATH, SERVICE).await?;
    let mut signals = proxy.receive_all_signals().await?;
    let capabilities: Vec<String> = timeout(TIMEOUT, proxy.call("GetCapabilities", &())).await??;
    let markup = capabilities.iter().any(|c| c == "body-markup");
    let client = Client::connect_to(path.to_path_buf()).await?;
    let connected_at = now_ms();
    let mut sub = timeout(TIMEOUT, client.subscribe()).await??;
    let initial_accounts = std::mem::take(&mut sub.messaging_accounts);
    let initial_conversations = std::mem::take(&mut sub.conversations);
    let (tx, mut rx) = mpsc::channel(32);
    let reader = Reader(tokio::spawn(async move {
        let _ = tx
            .send(Update::Snapshot(initial_accounts, initial_conversations))
            .await;
        while let Ok(message) = sub.next_message().await {
            if tx
                .send(Update::Payload(Box::new(message.payload)))
                .await
                .is_err()
            {
                break;
            }
        }
    }));
    let mut accounts = HashMap::<_, MessagingAccount>::new();
    let mut conversations = HashMap::<_, Conversation>::new();
    let mut seen = VecDeque::<MessageId>::new();
    let mut visible = VecDeque::<u32>::new();
    let mut ids = HashMap::<u32, (String, String)>::new();
    let outcome = async {
        loop { tokio::select! {
            update = rx.recv() => match update {
                Some(Update::Snapshot(a,c)) => { accounts = a.into_iter().map(|x|(x.id.clone(),x)).collect(); conversations = c.into_iter().map(|x|(x.id.clone(),x)).collect(); }
                Some(Update::Payload(payload)) => match *payload {
                    ServerPayload::AccountAdded{account}|ServerPayload::AccountUpdated{account} => { accounts.insert(account.id.clone(),account); },
                    ServerPayload::AccountRemoved{account_id} => { accounts.remove(&account_id); let account_id = account_id.to_string(); close_matching(&proxy, &mut visible, &mut ids, |(account, _)| account == &account_id).await; },
                    ServerPayload::ConversationAdded{conversation}|ServerPayload::ConversationUpdated{conversation} => { conversations.insert(conversation.id.clone(),conversation); },
                    ServerPayload::ConversationRemoved{conversation_id} => { conversations.remove(&conversation_id); let account_id = conversation_id.account_id.to_string(); close_matching(&proxy, &mut visible, &mut ids, |(account, thread)| account == &account_id && thread == &conversation_id.local_id).await; },
                    ServerPayload::MessageAdded{message} => {
                        let Some(ts)=message.sent_at else { continue };
                        // Normalized message timestamps are Unix microseconds.
                        let sent_at_ms = ts / 1_000;
                        if message.deleted || message.sender.is_self || sent_at_ms < connected_at || sent_at_ms > now_ms()+60_000 || seen.contains(&message.id) { continue; }
                        let Some(account)=accounts.get(&message.id.conversation_id.account_id).filter(|a|a.connected && a.authenticated) else {continue};
                        let title=conversations.get(&message.id.conversation_id).and_then(|c|c.title.clone()).or(message.sender.display_name.clone()).unwrap_or_else(||"New message".into());
                        let body=message.text.as_deref().filter(|s|!s.is_empty()).unwrap_or("New message");
                        let escaped=if markup {body.replace('&',"&amp;").replace('<',"&lt;").replace('>',"&gt;")} else {body.to_string()};
                        let mut hints=HashMap::<&str,Value<'_>>::new(); hints.insert("desktop-entry","handover-messages".into());
                        let id:u32=timeout(TIMEOUT,proxy.call("Notify",&(format!("Handover · {}",account.label),0u32,"mail-message-new",title,escaped,vec!["default".to_string(),"Open conversation".to_string(),"reply".to_string(),"Reply".to_string()],hints,-1i32))).await??;
                        if id>0 { seen.push_back(message.id.clone()); if seen.len()>SEEN {seen.pop_front();} ids.insert(id,(message.id.conversation_id.account_id.to_string(),message.id.conversation_id.local_id.clone())); visible.push_back(id); while visible.len()>VISIBLE { if let Some(old)=visible.pop_front(){ids.remove(&old); visible.retain(|id| *id != old); let _=timeout(TIMEOUT,proxy.call::<_,_,()>("CloseNotification",&(old,))).await;} } }
                    },
                    _=>{}
                },
                None=>return Err::<(),Box<dyn std::error::Error+Send+Sync>>("daemon disconnected".into()),
            },
            signal=signals.next()=>{let signal=signal.ok_or("notification service disconnected")?; match signal.header().member().map(|m|m.as_str().to_owned()).as_deref() { Some("ActionInvoked") => {let (id,key):(u32,String)=signal.body().deserialize()?; if let Some((account,thread))=action_target(id,&key,&ids) { launch_with(launcher,&account,&thread).await;}}, Some("NotificationClosed") => {let (id,_reason):(u32,u32)=signal.body().deserialize()?; ids.remove(&id); visible.retain(|visible_id| *visible_id != id);}, _ => {} }},
            _=owners.next()=>return Err("notification service owner changed".into()),
            changed=shutdown.changed()=>if changed.is_err()||*shutdown.borrow(){return Ok(())},
        }}
    }.await;
    for id in visible {
        let _ = timeout(TIMEOUT, proxy.call::<_, _, ()>("CloseNotification", &(id,))).await;
    }
    drop(reader);
    outcome
}

async fn close_matching(
    proxy: &Proxy<'_>,
    visible: &mut VecDeque<u32>,
    ids: &mut HashMap<u32, (String, String)>,
    mut matches: impl FnMut(&(String, String)) -> bool,
) {
    let closing: Vec<_> = ids
        .iter()
        .filter_map(|(id, target)| matches(target).then_some(*id))
        .collect();
    for id in closing {
        ids.remove(&id);
        visible.retain(|visible_id| *visible_id != id);
        let _ = timeout(TIMEOUT, proxy.call::<_, _, ()>("CloseNotification", &(id,))).await;
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn action_target(
    id: u32,
    key: &str,
    ids: &HashMap<u32, (String, String)>,
) -> Option<(String, String)> {
    matches!(key, "default" | "reply")
        .then(|| ids.get(&id).cloned())
        .flatten()
}

async fn launch_with(exe: &std::path::Path, account: &str, thread: &str) {
    let _ = Command::new(exe)
        .args(["--account", account, "--conversation", thread])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    use handover_core::{
        ConversationId, ConversationKind, Message, MessagingAccountId, Participant, TransportKind,
    };
    use handover_ipc::{Method, Request, ServerMessage, read_json_line, write_json_line};
    use std::sync::atomic::{AtomicU32, Ordering};
    use tokio::{
        io::{AsyncBufReadExt, BufReader},
        net::{UnixListener, UnixStream},
        process::{Child, Command as TokioCommand},
    };
    use zbus::{interface, object_server::SignalEmitter, zvariant::OwnedValue};

    const LIMIT: std::time::Duration = std::time::Duration::from_secs(3);
    #[derive(Debug)]
    enum Event {
        Posted(u32, String, String),
        Closed(u32),
    }
    struct NotifyService {
        next: AtomicU32,
        tx: mpsc::Sender<Event>,
    }
    #[interface(name = "org.freedesktop.Notifications")]
    impl NotifyService {
        fn get_capabilities(&self) -> Vec<&str> {
            vec!["body", "actions"]
        }
        #[allow(clippy::too_many_arguments)]
        async fn notify(
            &self,
            _app: &str,
            _replace: u32,
            _icon: &str,
            title: &str,
            body: &str,
            _actions: Vec<String>,
            _hints: HashMap<String, OwnedValue>,
            _timeout: i32,
        ) -> u32 {
            let id = self.next.fetch_add(1, Ordering::SeqCst);
            self.tx
                .send(Event::Posted(id, title.into(), body.into()))
                .await
                .unwrap();
            id
        }
        async fn close_notification(
            &self,
            id: u32,
            #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        ) {
            let _ = self.tx.send(Event::Closed(id)).await;
            Self::notification_closed(&emitter, id, 3).await.unwrap();
        }
        #[zbus(signal)]
        async fn action_invoked(
            emitter: &SignalEmitter<'_>,
            id: u32,
            key: &str,
        ) -> zbus::Result<()>;
        #[zbus(signal)]
        async fn notification_closed(
            emitter: &SignalEmitter<'_>,
            id: u32,
            reason: u32,
        ) -> zbus::Result<()>;
    }
    struct PrivateBus(Child);
    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.0.start_kill();
        }
    }
    async fn private_bus() -> (PrivateBus, String) {
        let mut child = TokioCommand::new("dbus-daemon")
            .args(["--session", "--nofork", "--nopidfile", "--print-address"])
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let mut addr = String::new();
        timeout(LIMIT, reader.read_line(&mut addr))
            .await
            .unwrap()
            .unwrap();
        (PrivateBus(child), addr.trim().to_owned())
    }
    async fn daemon(
        listener: &UnixListener,
        accounts: Vec<MessagingAccount>,
        conversations: Vec<Conversation>,
    ) -> UnixStream {
        let (mut socket, _) = timeout(LIMIT, listener.accept()).await.unwrap().unwrap();
        let request: Request = read_json_line(&mut BufReader::new(&mut socket))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            request.method,
            Method::Subscribe {
                shares: true,
                media: true,
                messages: true
            }
        );
        write_json_line(
            &mut socket,
            &ServerMessage::new(ServerPayload::Subscribed {
                devices: vec![],
                notifications: vec![],
                media_sessions: vec![],
                calls: vec![],
                messaging_accounts: accounts,
                conversations,
                typing_states: vec![],
                read_states: vec![],
            }),
        )
        .await
        .unwrap();
        socket
    }
    fn account() -> MessagingAccount {
        MessagingAccount {
            id: MessagingAccountId::new("fixture"),
            label: "Fixture".into(),
            connected: true,
            authenticated: true,
        }
    }
    fn conversation() -> Conversation {
        Conversation {
            id: ConversationId::new(MessagingAccountId::new("fixture"), "thread"),
            kind: ConversationKind::Direct,
            transport: TransportKind::Sms,
            title: Some("Known thread".into()),
            participants: vec![],
            latest_message_id: None,
            last_activity_at: None,
            unread_count: None,
            cursor: None,
            capabilities: Default::default(),
        }
    }
    fn message(id: &str, ts: i64, self_sender: bool) -> Message {
        Message {
            id: handover_core::MessageId::new(conversation().id, id),
            sender: Participant {
                local_id: "sender".into(),
                display_name: Some("Sender".into()),
                address: None,
                is_self: self_sender,
            },
            transport: None,
            sent_at: Some(ts),
            text: Some(format!("body-{id}")),
            attachments: vec![],
            reply_to: None,
            reactions: vec![],
            deleted: false,
        }
    }
    async fn next(rx: &mut mpsc::Receiver<Event>) -> Event {
        timeout(LIMIT, rx.recv()).await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn only_live_incoming_attested_messages_notify_and_default_action_targets_thread() {
        let (_bus, address) = private_bus().await;
        let (tx, mut rx) = mpsc::channel(16);
        let service = zbus::connection::Builder::address(address.as_str())
            .unwrap()
            .name(SERVICE)
            .unwrap()
            .serve_at(
                PATH,
                NotifyService {
                    next: AtomicU32::new(1),
                    tx,
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
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("daemon.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        let (stop, mut shutdown) = watch::channel(false);
        let bin = dir.path().join("bin");
        tokio::fs::create_dir(&bin).await.unwrap();
        let capture = dir.path().join("argv");
        let script = bin.join("handover-messages");
        tokio::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
                capture.display()
            ),
        )
        .await
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .await
            .unwrap();
        let worker = tokio::spawn({
            let connection = connection.clone();
            let socket_path = socket_path.clone();
            let launcher = script.clone();
            async move { session(&socket_path, &launcher, &connection, &mut shutdown).await }
        });
        let mut peer = daemon(&listener, vec![account()], vec![conversation()]).await;
        let emitter = service
            .object_server()
            .interface::<_, NotifyService>(PATH)
            .await
            .unwrap();
        let recent = now_ms() * 1_000;
        for rejected in [
            message("self", recent, true),
            message("unknown-time", -1, false),
            message("history", (recent / 1_000 - 120_000) * 1_000, false),
        ] {
            write_json_line(
                &mut peer,
                &ServerMessage::new(ServerPayload::MessageAdded { message: rejected }),
            )
            .await
            .unwrap();
        }
        write_json_line(
            &mut peer,
            &ServerMessage::new(ServerPayload::MessageAdded {
                message: message("incoming", recent, false),
            }),
        )
        .await
        .unwrap();
        let (id, title, body) = match next(&mut rx).await {
            Event::Posted(id, title, body) => (id, title, body),
            other => panic!("{other:?}"),
        };
        assert_eq!(title, "Known thread");
        assert_eq!(body, "body-incoming");
        assert_eq!(
            action_target(
                id,
                "default",
                &HashMap::from([(id, ("fixture".into(), "thread".into()))])
            ),
            Some(("fixture".into(), "thread".into()))
        );
        assert_eq!(
            action_target(
                id,
                "reply",
                &HashMap::from([(id, ("fixture".into(), "thread".into()))])
            ),
            Some(("fixture".into(), "thread".into()))
        );
        NotifyService::action_invoked(emitter.signal_emitter(), id, "default")
            .await
            .unwrap();
        timeout(LIMIT, async {
            while !capture.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            tokio::fs::read_to_string(&capture).await.unwrap(),
            "--account\nfixture\n--conversation\nthread\n"
        );
        stop.send(true).unwrap();
        assert!(matches!(next(&mut rx).await,Event::Closed(closed) if closed==id));
        timeout(LIMIT, worker).await.unwrap().unwrap().unwrap();
    }
}
