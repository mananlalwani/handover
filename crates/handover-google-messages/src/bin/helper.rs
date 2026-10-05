//! `handover-google-messages-helper`: the daemon-supervised helper process for
//! Handover's independently authored native Google Messages client.
//!
//! License boundary: this binary is MIT and first-party. It contains no
//! AGPL source, no `mautrix-gmessages` code, and no generated Google protobuf
//! definitions. It speaks the coarse normalized contract from
//! [`handover_gmessages::contract`] on stdin/stdout and keeps its own secrets
//! below that boundary. See `docs/gmessages-sidecar.md`.
//!
//! Scope: this is the process seam, not a finished client. It owns the
//! restricted session store, restores pending unpaired registrations, and
//! runs explicit bounded phone-pairing attempts, and gates messaging until a
//! usable session exists. Verified authentication stays in desktop Secret Service.
//!
//! It never logs bundles, tokens, keys, account email, message bodies, or
//! media bytes.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[cfg(test)]
use base64::Engine as _;
use handover_core::messaging::check_account_id;
use handover_gmessages::contract::{
    HELPER_PROTOCOL, HelperCommand, HelperEvent, MAX_BUNDLE_BYTES, MAX_HELPER_LINE_BYTES,
    NATIVE_GOOGLE_MESSAGES_HELPER_NAME,
};
use handover_google_messages::session_store::SessionStore;
use handover_google_messages::{
    login::{ConfirmedPairing, LoginBootstrap, LoginOutcome, LoginProgress},
    registration::UnpairedRegistration,
};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::{sync::mpsc, task::JoinSet};
use zeroize::{Zeroize, Zeroizing};

/// Reported in `Hello`. The daemon logs the OS process name separately; this
/// identifies the implementation, not the account.
const HELPER_NAME: &str = NATIVE_GOOGLE_MESSAGES_HELPER_NAME;

/// Label for a saved registration that has no confirmed phone pairing.
const ACCOUNT_LABEL: &str = "Google Messages (not paired)";

/// Stable reason for any command this helper cannot serve. Deliberately a
/// constant: per-command prose would leak account and message material into
/// the daemon's error path.
const UNAVAILABLE: &str = "capability not available in the native helper";

#[tokio::main]
async fn main() {
    let operation = std::env::args().nth(1);
    if matches!(
        operation.as_deref(),
        Some(
            "--probe-startup"
                | "--check-recovery"
                | "--probe-conversations"
                | "--probe-history"
                | "--probe-updates"
        )
    ) {
        let result = async {
            let store = SessionStore::default_store()
                .map_err(|_| handover_google_messages::ProbeError::SessionStoreFailed)?;
            let mut pairings = ConfirmedPairing::restore_all(&store)?;
            if pairings.len() != 1 {
                return Err(handover_google_messages::ProbeError::AmbiguousRegistration);
            }
            let session =
                handover_google_messages::session::RecoveredSession::restore(pairings.remove(0))
                    .await?;
            if operation.as_deref() == Some("--check-recovery") {
                Ok(None)
            } else if operation.as_deref() == Some("--probe-updates") {
                let known = session.read_conversations().await?;
                println!(
                    "Native update observer started for 60 seconds. No pushes will be acknowledged."
                );
                let mut count = 0;
                session
                    .observe_updates(known, std::time::Duration::from_secs(60), |update| {
                        use handover_google_messages::updates::Update;
                        match update {
                            Update::Conversations(records) => println!(
                                "Authenticated conversation update: {} record(s).",
                                records.len()
                            ),
                            Update::Messages(records) => println!(
                                "Authenticated message update: {} record(s).",
                                records.len()
                            ),
                            Update::Active => {
                                println!("Authenticated current-session activation observed.")
                            }
                            Update::Inactive => {
                                println!("Authenticated current-session preemption observed.")
                            }
                            Update::PresenceCheck => println!(
                                "Authenticated presence check observed; no presence response sent."
                            ),
                            Update::Unsupported(kind) => {
                                println!("Authenticated unhandled update category: {kind}.")
                            }
                        }
                        count += 1;
                        Ok(())
                    })
                    .await?;
                Ok(Some(count))
            } else if operation.as_deref() == Some("--probe-history") {
                let conversation = session
                    .read_conversations()
                    .await?
                    .into_iter()
                    .filter(|record| {
                        record
                            .participants
                            .iter()
                            .any(|participant| participant.is_self)
                            && record
                                .participants
                                .iter()
                                .any(|participant| !participant.is_self)
                    })
                    .max_by_key(|record| record.last_activity_at)
                    .ok_or(handover_google_messages::ProbeError::UnexpectedResponse)?;
                let first = session.read_history(&conversation, None, 20).await?;
                println!(
                    "Native history first page decoded and acknowledged: {} message(s).",
                    first.messages.len()
                );
                let mut count = first.messages.len();
                if let Some(cursor) = first.cursor_next.as_deref() {
                    let second = session
                        .read_history(&conversation, Some(cursor), 20)
                        .await?;
                    count += second.messages.len();
                    println!(
                        "Native older history page decoded and acknowledged: {} message(s).",
                        second.messages.len()
                    );
                }
                Ok(Some(count))
            } else if operation.as_deref() == Some("--probe-conversations") {
                session.probe_conversations().await.map(Some)
            } else {
                session.probe_startup().await.map(|_| None)
            }
        }
        .await;
        let failed = result.is_err();
        match result {
            Ok(None) if operation.as_deref() == Some("--check-recovery") => println!(
                "Native keys and desktop authentication restored; registration token is locally valid. No network request was sent."
            ),
            Ok(Some(count)) if operation.as_deref() == Some("--probe-updates") => println!(
                "Native update observer complete: {count} validated push(es). No contents or identifiers were printed, and no pushes were acknowledged."
            ),
            Ok(Some(count)) if operation.as_deref() == Some("--probe-history") => println!(
                "Native history decoded and acknowledged: {count} normalized message(s). No content or identifiers were printed."
            ),
            Ok(Some(count)) => println!(
                "Native conversations decoded and acknowledged: {count} normalized record(s). Full snapshot and ongoing messaging remain unverified."
            ),
            Ok(None) => println!(
                "Native receive opened and activation was HTTP accepted. Online messaging remains unverified."
            ),
            Err(handover_google_messages::ProbeError::SessionProtocol(category)) => println!(
                "Native startup failed (session_protocol, {category:?}). Saved pairing retained."
            ),
            Err(handover_google_messages::ProbeError::ReceiveProtocol(category)) => println!(
                "Native startup failed (receive_protocol, {category:?}). Saved pairing retained."
            ),
            Err(error) => println!(
                "Native startup failed ({}, HTTP {:?}, RPC {:?}). Saved pairing retained.",
                error.code(),
                error.http_status(),
                error.rpc_status()
            ),
        }
        if failed {
            std::process::exit(1);
        }
        return;
    }
    let mut reader = tokio::io::BufReader::new(tokio::io::stdin());
    let mut writer = tokio::io::stdout();
    let _ = serve_async(&mut NativeHelper::new(), &mut reader, &mut writer).await;
}

enum WorkOutcome {
    Login(LoginOutcome),
}

use handover_google_messages::session::{LiveCommand, LiveEvent, LiveOutput};

enum LiveSignal {
    Output(LiveOutput),
    Ended(Result<(), handover_google_messages::ProbeError>),
}
fn reconnect_delay(
    error: &handover_google_messages::ProbeError,
    attempt: u8,
) -> Option<std::time::Duration> {
    use handover_google_messages::ProbeError;
    let transient = matches!(
        error,
        ProbeError::Network | ProbeError::Timeout | ProbeError::ReceiveFailed
    ) || error
        .http_status()
        .is_some_and(|status| (500..=599).contains(&status));
    (transient && attempt < 5).then(|| std::time::Duration::from_secs(1 << attempt))
}

struct LiveWorker {
    generation: u64,
    ready: bool,
    commands: mpsc::Sender<LiveCommand>,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Drop for LiveWorker {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
impl LiveWorker {
    async fn stop(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}
fn start_live_worker(
    account: String,
    generation: u64,
    store: SessionStore,
    signals: mpsc::Sender<(u64, String, LiveSignal)>,
) -> LiveWorker {
    let (commands, incoming) = mpsc::channel(1);
    let task = tokio::spawn(async move {
        let result = async {
            let pairing = ConfirmedPairing::restore_all(&store)?.into_iter()
                .find(|pairing| pairing.account_id() == account)
                .ok_or(handover_google_messages::ProbeError::InvalidBootstrap)?;
            let session = handover_google_messages::session::RecoveredSession::restore(pairing).await?;
            let (events, mut outgoing) = mpsc::channel(1);
            let network = session.run_live(incoming, events);
            tokio::pin!(network);
            loop {
                tokio::select! {
                    result = &mut network => return result,
                    output = outgoing.recv() => {
                        let Some(output) = output else { return Err(handover_google_messages::ProbeError::NativeError); };
                        signals.send((generation, account.clone(), LiveSignal::Output(output))).await
                            .map_err(|_| handover_google_messages::ProbeError::NativeError)?;
                    }
                }
            }
        }.await;
        let _ = signals
            .send((generation, account, LiveSignal::Ended(result)))
            .await;
    });
    LiveWorker {
        generation,
        ready: false,
        commands,
        task: Some(task),
    }
}

fn wire_conversation(
    conversation: handover_core::messaging::Conversation,
) -> handover_gmessages::contract::WireConversation {
    use handover_core::messaging::ConversationKind;
    use handover_gmessages::contract::{
        WireConversation, WireConversationKind, WireParticipant, WireTransport,
    };
    WireConversation {
        local_id: conversation.id.local_id,
        kind: match conversation.kind {
            ConversationKind::Direct => WireConversationKind::Direct,
            ConversationKind::Group => WireConversationKind::Group,
        },
        transport: WireTransport::Unknown,
        title: conversation.title,
        participants: conversation
            .participants
            .into_iter()
            .map(|participant| WireParticipant {
                local_id: participant.local_id,
                display_name: participant.display_name,
                address: participant.address,
                is_self: participant.is_self,
            })
            .collect(),
        latest_message: None,
        last_activity_at: conversation.last_activity_at,
        unread_count: conversation.unread_count,
        cursor: None,
        capabilities: conversation
            .capabilities
            .contains(&handover_core::messaging::MessagingCapability::Text)
            .then(|| "text".to_owned())
            .into_iter()
            .collect(),
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CursorLink {
    account: String,
    conversation: String,
    message: String,
    native: String,
}
impl Drop for CursorLink {
    fn drop(&mut self) {
        self.account.zeroize();
        self.conversation.zeroize();
        self.message.zeroize();
        self.native.zeroize();
    }
}
const MAX_CURSOR_LINKS: usize = 128;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedCursorLinks {
    version: u32,
    links: VecDeque<CursorLink>,
}
const MAX_CURSOR_BYTES: usize = 512 * 1024;

struct HistoryRequest {
    account: String,
    conversation: String,
    limit: u32,
    cursor: Option<String>,
    fetch_id: Option<u64>,
}

async fn publish_history<W: AsyncWrite + Unpin>(
    writer: &mut W,
    account: String,
    conversation: String,
    fetch_id: Option<u64>,
    page: handover_google_messages::history::HistoryPage,
) -> std::io::Result<()> {
    publish_messages(writer, account, conversation, fetch_id, page, true).await
}

async fn publish_messages<W: AsyncWrite + Unpin>(
    writer: &mut W,
    account: String,
    conversation: String,
    fetch_id: Option<u64>,
    page: handover_google_messages::history::HistoryPage,
    history: bool,
) -> std::io::Result<()> {
    use handover_gmessages::contract::{WireAttachment, WireMessage};
    let mut event = HelperEvent::Messages {
        account,
        conversation,
        messages: Vec::new(),
        cursor_next: None,
        page_complete: false,
        full: false,
        generation: None,
        fetch_id,
    };
    for message in page.messages {
        let wire = WireMessage {
            local_id: message.id.local_id,
            sender: message.sender.local_id,
            transport: None,
            sent_at: message.sent_at,
            text: message.text,
            attachments: message
                .attachments
                .into_iter()
                .map(|item| WireAttachment {
                    local_id: item.local_id,
                    mime: item.mime,
                    name: item.name,
                    size_bytes: item.size_bytes,
                    staged_path: None,
                })
                .collect(),
            reply_to: None,
            reactions: Vec::new(),
            deleted: false,
        };
        let HelperEvent::Messages { messages, .. } = &mut event else {
            unreachable!()
        };
        messages.push(wire);
        if encode(&event).len() > MAX_HELPER_LINE_BYTES {
            let HelperEvent::Messages { messages, .. } = &mut event else {
                unreachable!()
            };
            let last = messages.pop().expect("just added");
            if messages.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "native message exceeds helper contract bound",
                ));
            }
            publish(writer, event.clone()).await?;
            let HelperEvent::Messages { messages, .. } = &mut event else {
                unreachable!()
            };
            messages.clear();
            messages.push(last);
        }
    }
    let HelperEvent::Messages {
        page_complete,
        cursor_next,
        ..
    } = &mut event
    else {
        unreachable!()
    };
    *page_complete = history;
    *cursor_next = page.cursor_next;
    if encode(&event).len() > MAX_HELPER_LINE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "native message page exceeds helper contract bound",
        ));
    }
    publish(writer, event).await
}

async fn publish_push_messages<W: AsyncWrite + Unpin>(
    writer: &mut W,
    account: String,
    records: Vec<handover_core::messaging::Message>,
) -> std::io::Result<()> {
    let mut threads = BTreeMap::<String, Vec<_>>::new();
    for record in records {
        threads
            .entry(record.id.conversation_id.local_id.clone())
            .or_default()
            .push(record);
    }
    for (conversation, messages) in threads {
        publish_messages(
            writer,
            account.clone(),
            conversation,
            None,
            handover_google_messages::history::HistoryPage {
                messages,
                cursor_next: None,
            },
            false,
        )
        .await?;
    }
    Ok(())
}

async fn publish_page<W: AsyncWrite + Unpin>(
    writer: &mut W,
    account: String,
    page: Vec<handover_core::messaging::Conversation>,
) -> std::io::Result<()> {
    let mut event = HelperEvent::Conversations {
        account,
        conversations: Vec::new(),
        full: false,
        generation: None,
    };
    for conversation in page {
        let HelperEvent::Conversations { conversations, .. } = &mut event else {
            unreachable!()
        };
        conversations.push(wire_conversation(conversation));
        if encode(&event).len() > MAX_HELPER_LINE_BYTES {
            let HelperEvent::Conversations { conversations, .. } = &mut event else {
                unreachable!()
            };
            let last = conversations.pop().expect("just added");
            if conversations.is_empty() {
                publish(
                    writer,
                    HelperEvent::Error {
                        message: "native conversation exceeds helper contract bound".into(),
                    },
                )
                .await?;
                continue;
            }
            publish(writer, event.clone()).await?;
            let HelperEvent::Conversations { conversations, .. } = &mut event else {
                unreachable!()
            };
            conversations.clear();
            conversations.push(last);
            if encode(&event).len() > MAX_HELPER_LINE_BYTES {
                let HelperEvent::Conversations { conversations, .. } = &mut event else {
                    unreachable!()
                };
                conversations.clear();
                publish(
                    writer,
                    HelperEvent::Error {
                        message: "native conversation exceeds helper contract bound".into(),
                    },
                )
                .await?;
            }
        }
    }
    if let HelperEvent::Conversations { conversations, .. } = &event {
        if !conversations.is_empty() {
            publish(writer, event).await?;
        }
    }
    Ok(())
}

async fn publish<W: AsyncWrite + Unpin>(writer: &mut W, event: HelperEvent) -> std::io::Result<()> {
    writer.write_all(encode(&event).as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await
}

struct TextSend {
    request_id: String,
    account: String,
    conversation: String,
    text: Zeroizing<String>,
    reply_to: Option<String>,
}
async fn queue_send<W: AsyncWrite + Unpin>(
    helper: &NativeHelper,
    workers: &BTreeMap<String, LiveWorker>,
    sends: &mut BTreeMap<String, (String, String)>,
    writer: &mut W,
    send: TextSend,
) -> std::io::Result<()> {
    use handover_core::messaging::{MAX_ID_LEN, MAX_TEXT_CHARS, MessagingCapability};
    let TextSend {
        request_id,
        account,
        conversation,
        text,
        reply_to,
    } = send;
    let allowed = helper.online.contains(&account)
        && !sends.contains_key(&account)
        && reply_to.is_none()
        && !request_id.is_empty()
        && request_id.len() <= MAX_ID_LEN
        && !request_id.chars().any(char::is_control)
        && !text.trim().is_empty()
        && !text.contains('\0')
        && text.chars().count() <= MAX_TEXT_CHARS
        && helper
            .conversations
            .get(&(account.clone(), conversation.clone()))
            .is_some_and(|model| model.capabilities.contains(&MessagingCapability::Text));
    let (accepted, gate) = tokio::sync::oneshot::channel();
    let queued = allowed
        && workers.get(&account).is_some_and(|worker| {
            worker
                .commands
                .try_send(LiveCommand::SendText {
                    request_id: request_id.clone(),
                    conversation: conversation.clone(),
                    text,
                    accepted: gate,
                })
                .is_ok()
        });
    if queued {
        sends.insert(account.clone(), (request_id.clone(), conversation));
    }
    publish(
        writer,
        HelperEvent::CommandResult {
            request_id,
            ok: queued,
            error: (!queued).then(|| UNAVAILABLE.into()),
        },
    )
    .await?;
    // Never submit before the normalized acceptance has reached the daemon pipe.
    if queued {
        let _ = accepted.send(());
    }
    Ok(())
}
async fn abandon_send<W: AsyncWrite + Unpin>(
    writer: &mut W,
    sends: &mut BTreeMap<String, (String, String)>,
    account: &str,
) -> std::io::Result<()> {
    if let Some((request_id, conversation)) = sends.remove(account) {
        publish(
            writer,
            HelperEvent::SendStatus {
                request_id,
                account: account.into(),
                conversation,
                message: None,
                status: "unknown".into(),
            },
        )
        .await?;
    }
    Ok(())
}
/// The command reader stays responsive throughout pairing. One ceremony and
/// eight progress events are allowed; pipe closure and logout cancel its I/O.
async fn serve_async<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin>(
    helper: &mut NativeHelper,
    reader: &mut R,
    writer: &mut W,
) -> std::io::Result<()> {
    let (progress_tx, mut progress_rx) = mpsc::channel(8);
    let mut jobs = JoinSet::new();
    let mut generation = 0_u64;
    let mut active_account: Option<String> = None;
    let (live_tx, mut live_rx) = mpsc::channel(8);
    let mut workers = BTreeMap::<String, LiveWorker>::new();
    let mut retries = BTreeMap::<String, tokio::time::Instant>::new();
    let mut attempts = BTreeMap::<String, u8>::new();
    let mut sends = BTreeMap::<String, (String, String)>::new();
    let mut line = Zeroizing::new(Vec::new());
    loop {
        if jobs.is_empty() {
            let requested = helper
                .pending_history
                .as_ref()
                .map(|request| request.account.clone())
                .or_else(|| helper.pending_sync.clone());
            if let Some(account) = requested {
                if !workers.contains_key(&account) {
                    retries.remove(&account);
                    attempts.remove(&account);
                    if workers.len() >= 8 {
                        helper.pending_history = None;
                        helper.pending_sync = None;
                        publish(
                            writer,
                            HelperEvent::Error {
                                message: "native live account limit reached".into(),
                            },
                        )
                        .await?;
                    } else if let Some(store) = helper.store.clone() {
                        generation = generation.wrapping_add(1);
                        workers.insert(
                            account.clone(),
                            start_live_worker(account.clone(), generation, store, live_tx.clone()),
                        );
                        // Every worker begins with its own bounded inventory read.
                        if helper.pending_sync.as_deref() == Some(&account) {
                            helper.pending_sync = None;
                        }
                    }
                }
            }
            if let Some(request) = helper.pending_history.take() {
                if let Some(worker) = workers.get(&request.account) {
                    if let Some(model) = helper
                        .conversations
                        .get(&(request.account.clone(), request.conversation.clone()))
                        .cloned()
                    {
                        let command = LiveCommand::History {
                            conversation: Box::new(model),
                            cursor: request
                                .cursor
                                .as_ref()
                                .map(|value| Zeroizing::new(value.clone())),
                            limit: request.limit,
                            fetch_id: request.fetch_id,
                        };
                        match worker.commands.try_send(command) {
                            Ok(()) => {}
                            Err(mpsc::error::TrySendError::Full(_)) => {
                                helper.pending_history = Some(request)
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => {
                                publish(
                                    writer,
                                    HelperEvent::Error {
                                        message: "native receive session unavailable".into(),
                                    },
                                )
                                .await?
                            }
                        }
                    } else if worker.ready {
                        publish(
                            writer,
                            HelperEvent::Error {
                                message: "native history conversation unavailable".into(),
                            },
                        )
                        .await?;
                    } else {
                        helper.pending_history = Some(request);
                    }
                } else {
                    publish(
                        writer,
                        HelperEvent::Error {
                            message: "native receive session unavailable".into(),
                        },
                    )
                    .await?;
                }
            }
            if let Some(account) = helper.pending_sync.take() {
                if let Some(worker) = workers.get(&account) {
                    match worker.commands.try_send(LiveCommand::Conversations) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            helper.pending_sync = Some(account)
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            publish(
                                writer,
                                HelperEvent::Error {
                                    message: "native receive session unavailable".into(),
                                },
                            )
                            .await?
                        }
                    }
                }
            }
        }
        let retry_deadline = retries.values().copied().min().unwrap_or_else(|| {
            tokio::time::Instant::now() + std::time::Duration::from_secs(86_400)
        });
        let mut bounded = AsyncReadExt::take(
            &mut *reader,
            MAX_LINE_READ.saturating_sub(line.len() as u64),
        );
        tokio::select! {
            read = bounded.read_until(b'\n', &mut line) => {
                if read? == 0 {
                    jobs.abort_all();
                    while jobs.join_next().await.is_some() {}
                    return Ok(());
                }
                let terminated = line.last() == Some(&b'\n');
                if terminated { line.pop(); }
                if line.len() <= MAX_HELPER_LINE_BYTES {
                    if let Ok(control) = serde_json::from_slice::<Control>(&line) {
                        if control.kind == "logout" {
                            if let Some(account) = control.account.as_deref() {
                                abandon_send(writer, &mut sends, account).await?;
                                retries.remove(account);
                                attempts.remove(account);
                                if let Some(worker) = workers.remove(account) {
                                    worker.stop().await;
                                    helper.online.remove(account);
                                    publish(writer, disconnected(account)).await?;
                                }
                            }
                        }
                        if control.kind == "logout" && active_account.is_some()
                            && control.account.as_deref() == active_account.as_deref() {
                            // Wait for cancellation before deleting credentials.
                            // A concurrently finishing task may be writing them.
                            jobs.abort_all();
                            while jobs.join_next().await.is_some() {}
                            generation = generation.wrapping_add(1);
                            active_account = None;
                            helper.login_active = None;
                            helper.pending_login = None;
                            helper.pending_sync = None;
                            helper.pending_history = None;
                        }
                        if control.kind == "logout" && helper.desktop_credentials {
                            if let Some(account) = control.account.as_deref().filter(|account| helper.accounts.contains(*account)) {
                                if handover_google_messages::credential_store::DesktopCredentialStore::delete(account).await.is_err() {
                                    publish(writer, HelperEvent::Error { message: "desktop credentials could not be forgotten".into() }).await?;
                                    line.zeroize();
                                    continue;
                                }
                            }
                        }
                    }
                }
                if !line.is_empty() && !line.iter().all(u8::is_ascii_whitespace) {
                    if line.len() <= MAX_HELPER_LINE_BYTES && let Ok(HelperCommand::SendText { request_id, account, conversation, text, reply_to }) = serde_json::from_slice::<HelperCommand>(&line) {
                        queue_send(helper, &workers, &mut sends, writer, TextSend { request_id, account, conversation, text: Zeroizing::new(text), reply_to }).await?;
                    } else {
                    for response in helper.handle_line(&line) {
                        writer.write_all(response.as_bytes()).await?;
                        writer.write_all(b"\n").await?;
                    }
                    writer.flush().await?;
                    }
                }
                line.zeroize();
                if helper.shutdown_requested() || !terminated {
                    jobs.abort_all();
                    while jobs.join_next().await.is_some() {}
                    return Ok(());
                }
                if let Some((account, bootstrap)) = helper.pending_login.take() {
                    abandon_send(writer, &mut sends, &account).await?;
                    retries.remove(&account);
                    attempts.remove(&account);
                    if let Some(worker) = workers.remove(&account) {
                        worker.stop().await;
                        helper.end_live_session(&account);
                        publish(writer, disconnected(&account)).await?;
                    }
                    generation = generation.wrapping_add(1);
                    let expected = generation;
                    active_account = Some(account.clone());
                    let progress_tx = progress_tx.clone();
                    jobs.spawn(async move {
                        let result = bootstrap.run(|progress| {
                            progress_tx.try_send((expected, account.clone(), progress))
                                .map_err(|_| handover_google_messages::ProbeError::NativeError)
                        }).await;
                        (expected, account, result.map(WorkOutcome::Login))
                    });
                }
            }
            _ = tokio::time::sleep_until(retry_deadline), if !retries.is_empty() && jobs.is_empty() => {
                let due: Vec<_> = retries.iter().filter(|(_, deadline)| **deadline <= tokio::time::Instant::now()).map(|(account, _)| account.clone()).collect();
                for account in due {
                    retries.remove(&account);
                    if helper.confirmed.contains_key(&account) && !workers.contains_key(&account) && workers.len() < 8 {
                        if let Some(store) = helper.store.clone() {
                            generation = generation.wrapping_add(1);
                            workers.insert(account.clone(), start_live_worker(account, generation, store, live_tx.clone()));
                        }
                    }
                }
            }
            Some((expected, account, signal)) = live_rx.recv() => {
                if workers.get(&account).is_none_or(|worker| worker.generation != expected) { continue; }
                match signal {
                    LiveSignal::Output(output) => {
                        match output.event {
                            LiveEvent::SendResult { request_id, conversation, message, status } => {
                                if sends.get(&account).is_some_and(|pending| pending.0 == request_id && pending.1 == conversation) {
                                    publish(writer, HelperEvent::SendStatus { request_id, account: account.clone(), conversation, message, status: status.into() }).await?;
                                    sends.remove(&account);
                                } else { continue; }
                            }
                            LiveEvent::Online => {
                                helper.online.insert(account.clone());
                                attempts.remove(&account);
                                publish(writer, HelperEvent::Account { account: account.clone(), label: "Google Messages".into(), connected: true, authenticated: true }).await?;
                            }
                            LiveEvent::Conversations(records) => {
                                helper.conversations.retain(|(owner, _), _| owner != &account);
                                if let Some(worker) = workers.get_mut(&account) { worker.ready = true; }
                                for record in &records { helper.conversations.insert((account.clone(), record.id.local_id.clone()), record.clone()); }
                                publish_page(writer, account.clone(), records).await?;
                            }
                            LiveEvent::ConversationUpdates(records) => {
                                for record in &records { helper.conversations.insert((account.clone(), record.id.local_id.clone()), record.clone()); }
                                publish_page(writer, account.clone(), records).await?;
                            }
                            LiveEvent::History { conversation, fetch_id, page } => {
                                helper.remember_cursor(&account, &conversation, &page);
                                if helper.save_cursor_links().is_err() {
                                    publish(writer, HelperEvent::Error { message: "native history cursor storage failed".into() }).await?;
                                    continue;
                                }
                                publish_history(writer, account.clone(), conversation, fetch_id, page).await?;
                            }
                            LiveEvent::Messages(records) => publish_push_messages(writer, account.clone(), records).await?,
                        }
                        let _ = output.accepted.send(());
                    }
                    LiveSignal::Ended(result) => {
                        abandon_send(writer, &mut sends, &account).await?;
                        if let Some(worker) = workers.remove(&account) { worker.stop().await; }
                        helper.end_live_session(&account);
                        publish(writer, disconnected(&account)).await?;
                        if let Err(error) = result {
                            let attempt = *attempts.get(&account).unwrap_or(&0);
                            if let Some(delay) = reconnect_delay(&error, attempt) {
                                attempts.insert(account.clone(), attempt + 1);
                                retries.insert(account.clone(), tokio::time::Instant::now() + delay);
                            }
                            let category = match error {
                                handover_google_messages::ProbeError::SessionProtocol(category) => format!("session_protocol, {category:?}"),
                                handover_google_messages::ProbeError::ReceiveProtocol(category) => format!("receive_protocol, {category:?}"),
                                _ => error.code().to_owned(),
                            };
                            publish(writer, HelperEvent::Error { message: format!("native receive ended ({category})") }).await?;
                        }
                    }
                }
            }
            Some((expected, account, progress)) = progress_rx.recv() => {
                if expected != generation || active_account.as_deref() != Some(&account) { continue; }
                let prompt = match progress {
                    LoginProgress::Ready => "Native registration matches the signed-in account.".to_owned(),
                    LoginProgress::RegistrationVerified => "Native registration matches the signed-in account. Opening pairing channel.".to_owned(),
                    LoginProgress::InitialSendAccepted => "Initial pairing request received HTTP acceptance. Waiting for the phone response.".to_owned(),
                    LoginProgress::InitialAcknowledgementAccepted => "Initial pairing reply acknowledged. Sending the final pairing request.".to_owned(),
                    LoginProgress::FinalSendAccepted => "Final pairing request received HTTP acceptance. Waiting for phone confirmation.".to_owned(),
                    LoginProgress::Verification(symbol) => format!("Confirm {symbol} on your phone."),
                };
                publish(writer, HelperEvent::Pairing { account, prompt }).await?;
            }
            Some(completed) = jobs.join_next(), if !jobs.is_empty() => {
                let (expected, account, result) = match completed {
                    Ok(completed) => completed,
                    Err(_) => {
                        helper.login_active = None;
                        active_account = None;
                        publish(writer, HelperEvent::Error { message: "native operation interrupted".into() }).await?;
                        continue;
                    }
                };
                if expected != generation || active_account.as_deref() != Some(&account) { continue; }
                // Drain progress before completing, so a fast mock peer cannot
                // cause the verification prompt to be lost behind completion.
                while let Ok((event_generation, event_account, progress)) = progress_rx.try_recv() {
                    if event_generation != generation || event_account != account { continue; }
                    let prompt = match progress {
                        LoginProgress::Ready => "Native registration matches the signed-in account.".to_owned(),
                        LoginProgress::RegistrationVerified => "Native registration matches the signed-in account. Opening pairing channel.".to_owned(),
                        LoginProgress::InitialSendAccepted => "Initial pairing request received HTTP acceptance. Waiting for the phone response.".to_owned(),
                        LoginProgress::InitialAcknowledgementAccepted => "Initial pairing reply acknowledged. Sending the final pairing request.".to_owned(),
                    LoginProgress::FinalSendAccepted => "Final pairing request received HTTP acceptance. Waiting for phone confirmation.".to_owned(),
                    LoginProgress::Verification(symbol) => format!("Confirm {symbol} on your phone."),
                    };
                    publish(writer, HelperEvent::Pairing { account: account.clone(), prompt }).await?;
                }
                helper.login_active = None;
                active_account = None;
                match result {
                    Ok(WorkOutcome::Login(LoginOutcome::Ready)) => {}
                    Ok(WorkOutcome::Login(LoginOutcome::CredentialsSaved)) => {
                        helper.pending_sync = Some(account.clone());
                        publish(writer, HelperEvent::Pairing { account, prompt: "Native account verified. Google authentication saved in the desktop credential store. Messaging startup is pending.".into() }).await?;
                    }
                    Ok(WorkOutcome::Login(LoginOutcome::PhoneConfirmed { pairing, acknowledgement_accepted, credential_error })) => {
                        helper.confirmed.insert(account.clone(), *pairing);
                        if credential_error.is_none() { helper.pending_sync = Some(account.clone()); }
                        publish(writer, HelperEvent::Account {
                            account: account.clone(), label: "Google Messages (offline)".into(),
                            connected: false, authenticated: false,
                        }).await?;
                        let prompt = if acknowledgement_accepted {
                            "Phone confirmed native pairing and its keys were saved. Messaging startup is pending."
                        } else {
                            "Phone confirmed native pairing and its keys were saved. Acknowledgement failed; do not pair again."
                        };
                        let prompt = if let Some(error) = credential_error {
                            format!("{prompt} Google authentication was not saved ({error:?}); sign in again, do not pair again.")
                        } else { prompt.into() };
                        publish(writer, HelperEvent::Pairing { account, prompt }).await?;
                    }
                    Err(error) => {
                        publish(writer, HelperEvent::Pairing {
                            account,
                            prompt: login_failure_prompt(&error),
                        }).await?;
                    }
                }
            }
        }
    }
}

fn login_failure_prompt(error: &handover_google_messages::ProbeError) -> String {
    let mut diagnostic = error.code().to_owned();
    if let Some(status) = error.http_status() {
        diagnostic.push_str(&format!(", HTTP {status}"));
    }
    if let Some(status) = error.rpc_status() {
        diagnostic.push_str(&format!(", RPC {status:?}"));
    }
    if let Some(reason) = error.rpc_reason() {
        diagnostic.push_str(&format!(", reason {reason:?}"));
    }
    if let handover_google_messages::ProbeError::ReceiveProtocol(cause) = error {
        diagnostic.push_str(&format!(", framing {cause:?}"));
    }
    if let handover_google_messages::ProbeError::CredentialStore(cause) = error {
        return format!(
            "Native account verified, but Google authentication was not saved ({cause:?}). Sign in again; do not pair again."
        );
    }
    format!("Native login failed ({diagnostic}). Check phone pairing state before retrying.")
}

#[derive(serde::Deserialize)]
struct Control {
    #[serde(rename = "type")]
    kind: String,
    account: Option<String>,
}

/// One read is capped just past the contract's line limit, plus room for the
/// newline and a byte of overshoot that proves the line was too long.
const MAX_LINE_READ: u64 = (MAX_HELPER_LINE_BYTES + 2) as u64;

fn encode(event: &HelperEvent) -> String {
    serde_json::to_string(event).unwrap_or_else(|_| {
        // Unreachable for the fixed event set, but never emit an empty line:
        // the daemon reads newline-delimited JSON and treats a blank line as
        // end of input for that read.
        r#"{"type":"error","message":"event encoding failed"}"#.to_string()
    })
}

/// Build the announcement for one account this helper knows about.
///
/// Saved registration alone does not establish a live receive session.
fn announcement(account: &str) -> HelperEvent {
    HelperEvent::Account {
        account: account.to_string(),
        label: ACCOUNT_LABEL.into(),
        connected: false,
        authenticated: false,
    }
}

fn disconnected(account: &str) -> HelperEvent {
    HelperEvent::Account {
        account: account.to_owned(),
        label: "Google Messages (offline)".into(),
        connected: false,
        authenticated: false,
    }
}

struct NativeHelper {
    /// `None` when no absolute state directory is configured. The helper
    /// still answers `Hello` in that case; it simply has nothing to restore.
    store: Option<SessionStore>,
    /// Accounts this helper has announced, so `Logout` removes exactly what
    /// `Login` and `Hello` created.
    accounts: BTreeSet<String>,
    online: BTreeSet<String>,
    shutdown: bool,
    pending_login: Option<(String, LoginBootstrap)>,
    pending_sync: Option<String>,
    pending_history: Option<HistoryRequest>,
    cursor_links: VecDeque<CursorLink>,
    cursor_links_loaded: bool,
    conversations: BTreeMap<(String, String), handover_core::messaging::Conversation>,
    login_active: Option<String>,
    confirmed: BTreeMap<String, ConfirmedPairing>,
    desktop_credentials: bool,
}

impl NativeHelper {
    fn new() -> Self {
        Self {
            store: SessionStore::default_store().ok(),
            accounts: BTreeSet::new(),
            online: BTreeSet::new(),
            shutdown: false,
            pending_login: None,
            pending_sync: None,
            pending_history: None,
            cursor_links: VecDeque::new(),
            cursor_links_loaded: false,
            conversations: BTreeMap::new(),
            login_active: None,
            confirmed: BTreeMap::new(),
            desktop_credentials: true,
        }
    }

    fn shutdown_requested(&self) -> bool {
        self.shutdown
    }

    fn handle_line(&mut self, line: &[u8]) -> Vec<String> {
        // Bound before parsing so an oversized line costs nothing beyond the
        // read. The message names no content.
        if line.len() > MAX_HELPER_LINE_BYTES {
            return vec![encode(&HelperEvent::Error {
                message: "helper line too large".into(),
            })];
        }
        let command: HelperCommand = match serde_json::from_slice(line) {
            Ok(command) => command,
            Err(_) => {
                return vec![encode(&HelperEvent::Error {
                    message: "malformed command".into(),
                })];
            }
        };
        // Bundle size is enforced before decode cost grows.
        if let HelperCommand::Login { bundle_b64, .. } = &command {
            if bundle_b64.len() > MAX_BUNDLE_BYTES {
                return vec![encode(&HelperEvent::Error {
                    message: "credential bundle too large".into(),
                })];
            }
        }
        self.dispatch(command).iter().map(encode).collect()
    }

    fn dispatch(&mut self, command: HelperCommand) -> Vec<HelperEvent> {
        match command {
            HelperCommand::Hello => self.hello(),
            HelperCommand::Login {
                account,
                bundle_b64,
            } => self.login(&account, &Zeroizing::new(bundle_b64)),
            HelperCommand::Logout { account } => self.logout(&account),
            HelperCommand::Shutdown => {
                self.shutdown = true;
                Vec::new()
            }
            // One pending read coalesces repeated requests while network work
            // is active. Conversation reads remain incremental until atomic
            // snapshot reconciliation is implemented.
            HelperCommand::ListConversations { account } | HelperCommand::Sync { account } => {
                if self.desktop_credentials && self.confirmed.contains_key(&account) {
                    self.pending_sync = Some(account);
                }
                Vec::new()
            }
            HelperCommand::MarkRead { .. } | HelperCommand::Typing { .. } => Vec::new(),
            // Keep one pending history request. Unsupported or excess work
            // fails explicitly instead of leaving a daemon waiter open.
            HelperCommand::FetchHistory {
                account,
                conversation,
                limit,
                cursor,
                fetch_id,
            } => {
                if self.desktop_credentials
                    && self.confirmed.contains_key(&account)
                    && self.pending_history.is_none()
                    && (1..=200).contains(&limit)
                    && cursor.as_ref().is_none_or(|value| value.len() <= 2048)
                {
                    let cursor = match cursor {
                        Some(public) => match self.cursor_links.iter().rev().find(|link| {
                            link.account == account
                                && link.conversation == conversation
                                && link.message == public
                        }) {
                            Some(link) => Some(link.native.clone()),
                            None => {
                                return vec![HelperEvent::Error {
                                    message: "native history cursor is no longer available".into(),
                                }];
                            }
                        },
                        None => None,
                    };
                    self.pending_history = Some(HistoryRequest {
                        account,
                        conversation,
                        limit: limit.min(50),
                        cursor,
                        fetch_id,
                    });
                    Vec::new()
                } else {
                    vec![HelperEvent::Error {
                        message: UNAVAILABLE.into(),
                    }]
                }
            }
            HelperCommand::SendText { request_id, .. }
            | HelperCommand::SendMedia { request_id, .. }
            | HelperCommand::React { request_id, .. }
            | HelperCommand::DeleteMessage { request_id, .. }
            | HelperCommand::OpenConversation { request_id, .. } => {
                vec![HelperEvent::CommandResult {
                    request_id,
                    // Rejection, not acceptance: nothing was submitted, so no
                    // outgoing operation may be recorded as sent.
                    ok: false,
                    error: Some(UNAVAILABLE.into()),
                }]
            }
        }
    }

    fn hello(&mut self) -> Vec<HelperEvent> {
        let mut events = vec![HelperEvent::Hello {
            helper_protocol: HELPER_PROTOCOL,
            name: HELPER_NAME.into(),
        }];
        let Some(store) = &self.store else {
            events.push(HelperEvent::Error {
                message: "native session store unavailable".into(),
            });
            return events;
        };
        // A saved pending registration is announced so a restarted daemon shows
        // the account without asking the user to register again. It is not
        // authenticated: no phone is paired and nothing has been re-attested
        // with Google. The persisted random alias is valid
        // for the daemon account gate and discloses no provider identity.
        match ConfirmedPairing::restore_all(store) {
            Ok(confirmed) => {
                for pairing in confirmed {
                    let account = pairing.account_id().to_owned();
                    self.accounts.insert(account.clone());
                    self.confirmed.insert(account.clone(), pairing);
                    let online = self.online.contains(&account);
                    events.push(HelperEvent::Account {
                        account,
                        label: if online {
                            "Google Messages"
                        } else {
                            "Google Messages (offline)"
                        }
                        .into(),
                        connected: online,
                        authenticated: online,
                    });
                }
            }
            Err(_) => {
                events.push(HelperEvent::Error {
                    message: "native confirmed session store is unreadable".into(),
                });
                return events;
            }
        }
        match UnpairedRegistration::restore_all_pending_with_keys(store) {
            Ok(pending) => {
                for (_key, registration) in pending {
                    let account = registration.handover_account_id().to_owned();
                    if !self.confirmed.contains_key(&account) {
                        self.accounts.insert(account.clone());
                        events.push(announcement(&account));
                    }
                }
            }
            // Fail closed: a record this build cannot validate must not be
            // announced as a usable account.
            Err(_) => events.push(HelperEvent::Error {
                message: "native session store is unreadable".into(),
            }),
        }
        if self.restore_cursor_links().is_err() {
            events.push(HelperEvent::Error {
                message: "native history cursor cache unavailable".into(),
            });
        }
        events
    }

    fn login(&mut self, account: &str, bundle_b64: &str) -> Vec<HelperEvent> {
        if check_account_id(account).is_err() {
            return vec![HelperEvent::Error {
                message: "login rejected: invalid account name".into(),
            }];
        }
        if self.login_active.is_some() {
            return vec![HelperEvent::Error {
                message: "native operation already active for an account".into(),
            }];
        }
        let Some(store) = &self.store else {
            return vec![HelperEvent::Error {
                message: "native session store unavailable".into(),
            }];
        };
        let bootstrap = match LoginBootstrap::from_bundle(account, bundle_b64, store) {
            Ok(bootstrap) => bootstrap,
            Err(error) => {
                return vec![HelperEvent::Error {
                    message: format!("native login rejected ({})", error.code()),
                }];
            }
        };
        self.login_active = Some(account.to_owned());
        self.pending_login = Some((account.to_owned(), bootstrap));
        self.accounts.insert(account.to_owned());
        vec![announcement(account)]
    }

    fn save_cursor_links(&self) -> Result<(), ()> {
        let store = self
            .store
            .as_ref()
            .ok_or(())?
            .history_cursor_store()
            .map_err(|_| ())?;
        // Serialize borrowed entries so the in-memory index stays redacted and
        // each dropped disk buffer is erased.
        #[derive(serde::Serialize)]
        struct Borrowed<'a> {
            version: u32,
            links: &'a VecDeque<CursorLink>,
        }
        let bytes = Zeroizing::new(
            serde_json::to_vec(&Borrowed {
                version: 1,
                links: &self.cursor_links,
            })
            .map_err(|_| ())?,
        );
        if bytes.len() > MAX_CURSOR_BYTES {
            return Err(());
        }
        let record = handover_google_messages::session_store::SessionRecord::new(bytes.to_vec())
            .map_err(|_| ())?;
        store.store("index", &record).map_err(|_| ())
    }

    fn restore_cursor_links(&mut self) -> Result<(), ()> {
        if self.cursor_links_loaded {
            return Ok(());
        }
        self.cursor_links_loaded = true;
        let store = self
            .store
            .as_ref()
            .ok_or(())?
            .history_cursor_store()
            .map_err(|_| ())?;
        let Some(record) = store.load("index").map_err(|_| ())? else {
            return Ok(());
        };
        if record.as_bytes().len() > MAX_CURSOR_BYTES {
            return Err(());
        }
        let mut saved: SavedCursorLinks =
            serde_json::from_slice(record.as_bytes()).map_err(|_| ())?;
        if saved.version != 1 || saved.links.len() > MAX_CURSOR_LINKS {
            return Err(());
        }
        for link in &saved.links {
            if handover_core::messaging::check_account_id(&link.account).is_err()
                || [&link.conversation, &link.message].iter().any(|value| {
                    value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control)
                })
                || link.native.is_empty()
                || link.native.len() > 2048
            {
                return Err(());
            }
        }
        saved
            .links
            .retain(|link| self.confirmed.contains_key(&link.account));
        self.cursor_links = saved.links;
        Ok(())
    }

    fn remember_cursor(
        &mut self,
        account: &str,
        conversation: &str,
        page: &handover_google_messages::history::HistoryPage,
    ) {
        if let (Some(native), Some(floor)) = (
            page.cursor_next.as_ref(),
            page.messages.iter().min_by(|first, second| {
                (first.sent_at, &first.id.local_id).cmp(&(second.sent_at, &second.id.local_id))
            }),
        ) {
            self.cursor_links.retain(|link| {
                !(link.account == account
                    && link.conversation == conversation
                    && link.message == floor.id.local_id)
            });
            if self.cursor_links.len() == MAX_CURSOR_LINKS {
                self.cursor_links.pop_front();
            }
            self.cursor_links.push_back(CursorLink {
                account: account.into(),
                conversation: conversation.into(),
                message: floor.id.local_id.clone(),
                native: native.clone(),
            });
        }
    }

    fn clear_pending_reads(&mut self, account: &str) {
        if self
            .pending_history
            .as_ref()
            .is_some_and(|request| request.account == account)
        {
            self.pending_history = None;
        }
        if self.pending_sync.as_deref() == Some(account) {
            self.pending_sync = None;
        }
    }

    fn end_live_session(&mut self, account: &str) {
        self.online.remove(account);
        self.clear_pending_reads(account);
    }

    fn logout(&mut self, account: &str) -> Vec<HelperEvent> {
        if !self.accounts.contains(account) {
            return Vec::new();
        }
        self.clear_pending_reads(account);
        self.conversations.retain(|(owner, _), _| owner != account);
        self.cursor_links.retain(|link| link.account != account);
        // Stop this account's transient work before forgetting local credentials.
        // No remote revocation has been implemented.
        if self.login_active.as_deref() == Some(account) {
            self.login_active = None;
            self.pending_login = None;
        }
        if self.save_cursor_links().is_err() {
            return vec![HelperEvent::Error {
                message: "native history cursor cleanup failed".into(),
            }];
        }
        let forgotten = (|| {
            let store = self.store.as_ref().ok_or(())?;
            // Include a confirmation that completed before its actor event
            // was observed. Logout must remove the actual saved record.
            for pairing in ConfirmedPairing::restore_all(store).map_err(|_| ())? {
                if pairing.account_id() == account {
                    pairing.forget(store).map_err(|_| ())?;
                }
            }
            for (key, pending) in
                UnpairedRegistration::restore_all_pending_with_keys(store).map_err(|_| ())?
            {
                if pending.handover_account_id() == account {
                    store.delete(&key).map_err(|_| ())?;
                }
            }
            Ok::<_, ()>(())
        })();
        if forgotten.is_err() {
            return vec![HelperEvent::Error {
                message: "native credentials could not be forgotten".into(),
            }];
        }
        self.confirmed.remove(account);
        self.accounts.remove(account);
        self.online.remove(account);
        vec![HelperEvent::AccountRemoved {
            account: account.to_owned(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_failure_preserves_safe_http_diagnostics() {
        let error = handover_google_messages::ProbeError::HttpErrorWithStatus(
            400,
            handover_google_messages::RpcStatus::InvalidArgument,
        );
        let prompt = login_failure_prompt(&error);
        assert!(prompt.contains("http_error, HTTP 400, RPC InvalidArgument"));
        let invalid = login_failure_prompt(&handover_google_messages::ProbeError::HttpError(0));
        assert!(!invalid.contains("HTTP 0"));
        assert!(
            login_failure_prompt(&handover_google_messages::ProbeError::RpcErrorWithStatus(
                handover_google_messages::RpcStatus::Unauthenticated,
            ))
            .contains("rpc_error, RPC Unauthenticated")
        );
        assert!(
            login_failure_prompt(&handover_google_messages::ProbeError::ReceiveProtocol(
                handover_google_messages::receive::ReceiveError::Malformed,
            ))
            .contains("receive_failed, framing Malformed")
        );
    }

    fn account_ids(events: &[HelperEvent]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|event| match event {
                HelperEvent::Account { account, .. } => Some(account.as_str()),
                _ => None,
            })
            .collect()
    }

    fn line(helper: &mut NativeHelper, command: &HelperCommand) -> Vec<HelperEvent> {
        let encoded = serde_json::to_vec(command).unwrap();
        helper
            .handle_line(&encoded)
            .iter()
            .map(|raw| serde_json::from_str(raw).expect("helper emits valid JSON"))
            .collect()
    }

    #[test]
    fn credential_failure_does_not_invite_pairing_again() {
        let prompt = login_failure_prompt(&handover_google_messages::ProbeError::CredentialStore(
            handover_google_messages::credential_store::CredentialError::Locked,
        ));
        assert_eq!(
            prompt,
            "Native account verified, but Google authentication was not saved (Locked). Sign in again; do not pair again."
        );
    }

    #[tokio::test]
    async fn text_send_admission_and_abandonment_are_bounded_and_explicit() {
        use handover_core::messaging::{
            Conversation, ConversationId, ConversationKind, MessagingAccountId,
            MessagingCapability, Participant, TransportKind,
        };
        for case in [
            "accepted",
            "busy",
            "offline",
            "no_capability",
            "reply",
            "empty",
            "nul",
            "full",
            "closed",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut helper = helper_with_store(directory.path());
            helper.online.insert("work".into());
            let model = Conversation {
                id: ConversationId::new(MessagingAccountId::new("work"), "thread"),
                kind: ConversationKind::Direct,
                transport: TransportKind::Unknown,
                title: None,
                participants: vec![Participant {
                    local_id: "peer".into(),
                    display_name: None,
                    address: None,
                    is_self: false,
                }],
                latest_message_id: None,
                last_activity_at: None,
                unread_count: None,
                cursor: None,
                capabilities: BTreeSet::from([MessagingCapability::Text]),
            };
            helper
                .conversations
                .insert(("work".into(), "thread".into()), model);
            let (commands, mut incoming) = mpsc::channel(1);
            let mut workers = BTreeMap::new();
            workers.insert(
                "work".into(),
                LiveWorker {
                    generation: 1,
                    ready: true,
                    commands,
                    task: None,
                },
            );
            let mut sends = BTreeMap::new();
            if case == "busy" {
                sends.insert("work".into(), ("previous".into(), "thread".into()));
            }
            if case == "offline" {
                helper.online.clear();
            }
            if case == "no_capability" {
                helper
                    .conversations
                    .values_mut()
                    .next()
                    .unwrap()
                    .capabilities
                    .clear();
            }
            if case == "full" {
                workers["work"]
                    .commands
                    .try_send(LiveCommand::Conversations)
                    .unwrap_or_else(|_| panic!("fixture queue"));
            }
            if case == "closed" {
                incoming.close();
            }
            let mut output = Vec::new();
            queue_send(
                &helper,
                &workers,
                &mut sends,
                &mut output,
                TextSend {
                    request_id: "send-test".into(),
                    account: "work".into(),
                    conversation: "thread".into(),
                    text: Zeroizing::new(
                        match case {
                            "empty" => " ",
                            "nul" => "a\0b",
                            _ => "hello",
                        }
                        .into(),
                    ),
                    reply_to: (case == "reply").then(|| "message".into()),
                },
            )
            .await
            .unwrap();
            let event: HelperEvent = serde_json::from_slice(&output).unwrap();
            assert!(
                matches!(event, HelperEvent::CommandResult { ok, .. } if ok == (case=="accepted")),
                "case {case}"
            );
            if case == "accepted" {
                let LiveCommand::SendText { accepted, .. } = incoming.recv().await.unwrap() else {
                    panic!("queued send");
                };
                accepted.await.unwrap();
                output.clear();
                abandon_send(&mut output, &mut sends, "work").await.unwrap();
                let event: HelperEvent = serde_json::from_slice(&output).unwrap();
                assert!(
                    matches!(event, HelperEvent::SendStatus { status, message: None, .. } if status=="unknown")
                );
                assert!(sends.is_empty());
                output.clear();
                abandon_send(&mut output, &mut sends, "work").await.unwrap();
                assert!(output.is_empty(), "abandonment is reported once");
            } else {
                assert!(
                    sends
                        .get("work")
                        .is_none_or(|pending| pending.0 == "previous")
                );
                if case != "full" {
                    assert!(incoming.try_recv().is_err());
                }
            }
        }
    }

    #[tokio::test]
    async fn conversation_pages_keep_the_contract_bound_and_remain_incremental() {
        use handover_core::messaging::{
            Conversation, ConversationId, ConversationKind, MessagingAccountId, Participant,
            TransportKind,
        };
        let account = MessagingAccountId::new("gmessages-fixture");
        let mut page = Vec::new();
        for index in 0..12 {
            page.push(Conversation {
                id: ConversationId::new(account.clone(), format!("thread-{index}")),
                kind: ConversationKind::Group,
                transport: TransportKind::Unknown,
                title: Some("Fixture".into()),
                participants: (0..256)
                    .map(|participant| Participant {
                        local_id: format!("participant-{participant}"),
                        display_name: Some("N".repeat(256)),
                        address: Some("A".repeat(256)),
                        is_self: false,
                    })
                    .collect(),
                latest_message_id: None,
                last_activity_at: Some(1234),
                unread_count: None,
                cursor: None,
                capabilities: BTreeSet::new(),
            });
        }
        let mut output = Vec::new();
        publish_page(&mut output, account.as_str().into(), page)
            .await
            .unwrap();
        let mut count = 0;
        let mut chunks = 0;
        for line in output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            assert!(line.len() <= MAX_HELPER_LINE_BYTES);
            let HelperEvent::Conversations {
                conversations,
                full,
                generation,
                ..
            } = serde_json::from_slice(line).unwrap()
            else {
                panic!("page must contain only conversations")
            };
            assert!(!full);
            assert!(generation.is_none());
            for wire in conversations {
                let normalized =
                    handover_gmessages::normalize_conversation(&account, wire).unwrap();
                assert_eq!(normalized.transport, TransportKind::Unknown);
                assert!(normalized.capabilities.is_empty());
                count += 1;
            }
            chunks += 1;
        }
        assert_eq!(count, 12);
        assert!(chunks > 1);
    }

    fn helper_with_store(directory: &std::path::Path) -> NativeHelper {
        let mut helper = NativeHelper::new();
        helper.store = Some(SessionStore::new(directory.join("sessions")));
        helper.desktop_credentials = false;
        helper
    }

    fn saved_login(helper: &NativeHelper) -> (String, String) {
        let registration = handover_google_messages::registration::RegistrationAttempt::prepare()
            .unwrap()
            .accept_response(
                br#"[[],"c3ludGhldGljLWlk",null,["c3ludGhldGljLXRva2Vu","3600000000"]]"#,
                std::time::Duration::from_secs(3600),
            )
            .unwrap();
        let account = registration.handover_account_id().to_owned();
        registration
            .persist_pending(helper.store.as_ref().unwrap())
            .unwrap();
        let mut bytes = b"HOVL\x01\0".to_vec();
        bytes.extend(
            serde_json::to_vec(&serde_json::json!({
                "type": "gaia_login", "endpoint": "https://instantmessaging-pa.googleapis.com",
                "origin": "https://messages.google.com", "authorization": "Bearer synthetic",
                "api_key": "synthetic", "auth_user": "0", "service_cookie": "SID=synthetic",
                "account_email": "person@example.test",
            }))
            .unwrap(),
        );
        (
            account,
            base64::engine::general_purpose::STANDARD.encode(bytes),
        )
    }

    fn hello_events(helper: &mut NativeHelper) -> Vec<HelperEvent> {
        line(helper, &HelperCommand::Hello)
    }

    #[test]
    fn hello_reports_the_contract_version_and_nothing_else_when_empty() {
        let directory = tempfile::tempdir().unwrap();
        let mut helper = helper_with_store(directory.path());
        assert_eq!(
            hello_events(&mut helper),
            vec![HelperEvent::Hello {
                helper_protocol: HELPER_PROTOCOL,
                name: HELPER_NAME.into(),
            }]
        );
    }

    #[test]
    fn hello_announces_the_persisted_random_account_alias() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("sessions"));
        let response = br#"[[],"c3ludGhldGljLWlk",null,["c3ludGhldGljLXRva2Vu","3600000000"]]"#;
        let registration = handover_google_messages::registration::RegistrationAttempt::prepare()
            .unwrap()
            .accept_response(response, std::time::Duration::from_secs(3600))
            .unwrap();
        let account_id = registration.handover_account_id().to_string();
        let protocol_identity =
            handover_google_messages::session_store::account_key_for_identity(b"synthetic-id")
                .unwrap();
        registration.persist_pending(&store).unwrap();
        let mut helper = NativeHelper::new();
        helper.store = Some(store);

        let events = hello_events(&mut helper);
        assert_eq!(account_ids(&events), vec![account_id.as_str()]);
        assert!(!account_ids(&events).contains(&protocol_identity.as_str()));
        assert!(matches!(
            events
                .iter()
                .find(|event| matches!(event, HelperEvent::Account { .. })),
            Some(HelperEvent::Account {
                connected: false,
                authenticated: false,
                ..
            })
        ));
    }

    #[test]
    fn confirmed_records_restore_once_as_offline_and_logout_forgets_them() {
        let directory = tempfile::tempdir().unwrap();
        let mut helper = helper_with_store(directory.path());
        let (account, _) = saved_login(&helper);
        let store = helper.store.as_ref().unwrap().clone();
        let pending = store.load_all().unwrap();
        let (key, registration) = &pending[0];
        fn bytes(field: u32, value: &[u8], output: &mut Vec<u8>) {
            prost::encoding::encode_varint(u64::from((field << 3) | 2), output);
            prost::encoding::encode_varint(value.len() as u64, output);
            output.extend_from_slice(value);
        }
        // Synthetic local record, independent of the protocol crate's private
        // serializer. It carries no browser material or live pairing evidence.
        let mut pairing = vec![8, 1];
        bytes(2, &[0; 32], &mut pairing);
        bytes(3, &[1; 32], &mut pairing);
        bytes(4, b"synthetic-phone", &mut pairing);
        bytes(5, b"12345678-1234-4234-8234-123456789abc", &mut pairing);
        let mut record = vec![8, 1];
        bytes(2, registration.as_bytes(), &mut record);
        bytes(3, &pairing, &mut record);
        let confirmed_store = SessionStore::new(directory.path().join("sessions/confirmed"));
        confirmed_store
            .store(
                key,
                &handover_google_messages::session_store::SessionRecord::new(record).unwrap(),
            )
            .unwrap();
        let events = hello_events(&mut helper);
        assert_eq!(account_ids(&events), [account.as_str()]);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, HelperEvent::Account {
            connected: false, authenticated: false, label, ..
        } if label == "Google Messages (offline)"))
        );
        helper.online.insert(account.clone());
        assert!(hello_events(&mut helper).iter().any(|event| matches!(
            event,
            HelperEvent::Account {
                connected: true,
                authenticated: true,
                ..
            }
        )));
        helper.end_live_session(&account);
        assert!(hello_events(&mut helper).iter().any(|event| matches!(
            event,
            HelperEvent::Account {
                connected: false,
                authenticated: false,
                ..
            }
        )));
        helper.cursor_links.push_back(CursorLink {
            account: account.clone(),
            conversation: "thread".into(),
            message: "oldest".into(),
            native: "private-cursor".into(),
        });
        helper.save_cursor_links().unwrap();
        let mut recovered = helper_with_store(directory.path());
        hello_events(&mut recovered);
        assert_eq!(recovered.cursor_links.len(), 1);
        recovered.desktop_credentials = true;
        assert!(
            recovered
                .dispatch(HelperCommand::FetchHistory {
                    account: account.clone(),
                    conversation: "thread".into(),
                    limit: 100,
                    cursor: Some("oldest".into()),
                    fetch_id: Some(8),
                })
                .is_empty()
        );
        let queued = recovered.pending_history.as_ref().unwrap();
        assert_eq!(queued.cursor.as_deref(), Some("private-cursor"));
        assert_eq!(queued.limit, 50);
        assert_eq!(queued.fetch_id, Some(8));
        assert!(matches!(
            recovered
                .dispatch(HelperCommand::FetchHistory {
                    account: account.clone(),
                    conversation: "other-thread".into(),
                    limit: 20,
                    cursor: Some("oldest".into()),
                    fetch_id: Some(9),
                })
                .as_slice(),
            [HelperEvent::Error { .. }]
        ));
        assert_eq!(
            line(
                &mut helper,
                &HelperCommand::Logout {
                    account: account.clone()
                }
            ),
            [HelperEvent::AccountRemoved { account }]
        );
        assert!(store.load_all().unwrap().is_empty());
        assert!(confirmed_store.load_all().unwrap().is_empty());
        let mut restarted = helper_with_store(directory.path());
        assert!(account_ids(&hello_events(&mut restarted)).is_empty());
        assert!(restarted.cursor_links.is_empty());
    }

    #[test]
    fn hello_fails_closed_when_the_store_holds_an_unrecognized_record() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("sessions"));
        store
            .store(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                &handover_google_messages::session_store::SessionRecord::new(
                    b"not a registration".to_vec(),
                )
                .unwrap(),
            )
            .unwrap();
        let mut helper = NativeHelper::new();
        helper.store = Some(store);
        let events = hello_events(&mut helper);
        assert!(account_ids(&events).is_empty(), "no account is announced");
        assert!(
            events
                .iter()
                .any(|event| matches!(event, HelperEvent::Error { .. })),
            "the unreadable store is reported"
        );
    }

    #[test]
    fn login_bounds_the_bundle_and_leaves_the_store_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let mut helper = helper_with_store(directory.path());
        let store = helper.store.clone().expect("helper has a store");
        let (account, bundle) = saved_login(&helper);
        let before = store.load_all().unwrap();

        // Nothing browser-derived may reach disk, and the pre-existing record
        // must survive untouched. Count the directory itself rather than
        // `load_all`: that view skips records whose key is not a 64-character
        // digest, so it would miss a badly named write.
        let sessions = directory.path().join("sessions");
        let files = |path: &std::path::Path| {
            let mut names: Vec<String> = std::fs::read_dir(path)
                .map(|entries| {
                    entries
                        .filter_map(|entry| entry.ok())
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        };
        let before_files = files(&sessions);
        let events = line(
            &mut helper,
            &HelperCommand::Login {
                account: account.clone(),
                bundle_b64: bundle.clone(),
            },
        );
        assert_eq!(account_ids(&events), vec![account.as_str()]);

        let after_files = files(&sessions);
        assert_eq!(
            after_files, before_files,
            "a login writes, deletes, or renames no session record"
        );
        assert_eq!(
            before_files.len(),
            1,
            "the store was seeded with one record"
        );
        let after = store.load_all().unwrap();
        assert_eq!(before[0].0, after[0].0);
        assert_eq!(before[0].1.as_bytes(), after[0].1.as_bytes());

        for bad in ["", "not base64!!", &"A".repeat(MAX_BUNDLE_BYTES + 1)] {
            let rejected = line(
                &mut helper,
                &HelperCommand::Login {
                    account: account.clone(),
                    bundle_b64: bad.to_string(),
                },
            );
            assert!(
                rejected
                    .iter()
                    .all(|event| matches!(event, HelperEvent::Error { .. })),
                "bundle {bad:?} must be rejected"
            );
        }
        assert_eq!(
            files(&sessions),
            before_files,
            "a rejected login writes nothing either"
        );
    }

    #[test]
    fn saved_registrations_are_announced_offline() {
        // A pending registration has no paired phone and nothing has been
        // re-attested with Google. Announcing it as live would let a client
        // believe the account is usable.
        assert_eq!(
            announcement(&"a".repeat(64)),
            HelperEvent::Account {
                account: "a".repeat(64),
                label: ACCOUNT_LABEL.into(),
                connected: false,
                authenticated: false,
            }
        );
        // The same shape must come out of both account sources, not just the
        // helper that builds the event.
        let directory = tempfile::tempdir().unwrap();
        let mut helper = helper_with_store(directory.path());
        let (account, bundle) = saved_login(&helper);
        let announced = line(
            &mut helper,
            &HelperCommand::Login {
                account: account.clone(),
                bundle_b64: bundle,
            },
        );
        assert_eq!(announced, vec![announcement(&account)]);
    }

    #[test]
    fn login_rejects_account_names_the_daemon_would_refuse() {
        let directory = tempfile::tempdir().unwrap();
        let mut helper = helper_with_store(directory.path());
        for account in ["", "a/b", "a.b", ".."] {
            let events = line(
                &mut helper,
                &HelperCommand::Login {
                    account: account.to_string(),
                    bundle_b64: base64::engine::general_purpose::STANDARD.encode(b"x"),
                },
            );
            assert!(
                events
                    .iter()
                    .all(|event| matches!(event, HelperEvent::Error { .. })),
                "account {account:?} must be rejected"
            );
        }
        assert!(
            helper.accounts.is_empty(),
            "a rejected login announces no account"
        );
    }

    #[test]
    fn logout_removes_only_announced_accounts() {
        let directory = tempfile::tempdir().unwrap();
        let mut helper = helper_with_store(directory.path());
        assert!(
            line(
                &mut helper,
                &HelperCommand::Logout {
                    account: "ghost".into()
                }
            )
            .is_empty(),
            "an account this helper never announced is not removed"
        );
        let (account, bundle) = saved_login(&helper);
        line(
            &mut helper,
            &HelperCommand::Login {
                account: account.clone(),
                bundle_b64: bundle,
            },
        );
        assert_eq!(
            line(
                &mut helper,
                &HelperCommand::Logout {
                    account: account.clone()
                }
            ),
            vec![HelperEvent::AccountRemoved {
                account: account.clone(),
            }]
        );
        assert!(
            line(
                &mut helper,
                &HelperCommand::Logout {
                    account: account.clone()
                }
            )
            .is_empty()
        );
    }

    #[test]
    fn unserved_commands_reject_rather_than_claim_success() {
        let directory = tempfile::tempdir().unwrap();
        let mut helper = helper_with_store(directory.path());

        // A send must never look accepted: an accepted command would let the
        // daemon journal an outgoing operation that was never submitted.
        for (request_id, command) in [
            (
                "r1",
                HelperCommand::SendText {
                    request_id: "r1".into(),
                    account: "work".into(),
                    conversation: "c1".into(),
                    text: "hi".into(),
                    reply_to: None,
                },
            ),
            (
                "r2",
                HelperCommand::DeleteMessage {
                    request_id: "r2".into(),
                    account: "work".into(),
                    conversation: "c1".into(),
                    message: "m1".into(),
                },
            ),
        ] {
            assert_eq!(
                line(&mut helper, &command),
                vec![HelperEvent::CommandResult {
                    request_id: request_id.into(),
                    ok: false,
                    error: Some(UNAVAILABLE.into()),
                }]
            );
        }

        // No conversation list, window, or read state may be published: an
        // empty authoritative list would read as "this account is empty".
        for command in [
            HelperCommand::ListConversations {
                account: "work".into(),
            },
            HelperCommand::Sync {
                account: "work".into(),
            },
            HelperCommand::MarkRead {
                account: "work".into(),
                conversation: "c1".into(),
                message: None,
            },
            HelperCommand::Typing {
                account: "work".into(),
                conversation: "c1".into(),
            },
        ] {
            assert!(
                line(&mut helper, &command).is_empty(),
                "{command:?} must publish nothing"
            );
        }

        // A pending history request must fail its waiter rather than hang.
        assert!(matches!(
            line(
                &mut helper,
                &HelperCommand::FetchHistory {
                    account: "work".into(),
                    conversation: "c1".into(),
                    limit: 20,
                    cursor: None,
                    fetch_id: Some(7),
                }
            )[0],
            HelperEvent::Error { .. }
        ));
    }

    #[test]
    fn shutdown_is_terminal_and_malformed_input_is_dropped_without_echo() {
        let directory = tempfile::tempdir().unwrap();
        let mut helper = helper_with_store(directory.path());
        assert_eq!(
            helper.handle_line(b"{\"type\":\"unknown\"}"),
            vec![encode(&HelperEvent::Error {
                message: "malformed command".into(),
            })]
        );
        assert!(!helper.shutdown_requested());
        assert_eq!(
            line(&mut helper, &HelperCommand::Shutdown),
            Vec::<HelperEvent>::new()
        );
        assert!(helper.shutdown_requested());
    }

    #[test]
    fn oversized_lines_are_dropped_before_parsing() {
        let directory = tempfile::tempdir().unwrap();
        let mut helper = helper_with_store(directory.path());
        let oversized = vec![b'x'; MAX_HELPER_LINE_BYTES + 1];
        assert_eq!(
            helper.handle_line(&oversized),
            vec![encode(&HelperEvent::Error {
                message: "helper line too large".into(),
            })]
        );
        assert!(!helper.shutdown_requested());
    }

    /// Drive the real read path. `handle_line` alone would not catch an
    /// unbounded read, which is the failure this guards against.
    #[tokio::test]
    async fn serve_bounds_each_read_and_answers_pipelined_commands() {
        let directory = tempfile::tempdir().unwrap();
        let mut helper = helper_with_store(directory.path());
        let mut input: Vec<u8> = Vec::new();
        input.extend_from_slice(b"{\"type\":\"hello\"}\n");
        input.extend_from_slice(b"\n");
        input.extend_from_slice(b"   \n");
        input.extend_from_slice(b"{\"type\":\"nonsense\"}\n");
        input.extend_from_slice(b"{\"type\":\"shutdown\"}\n");
        input.extend_from_slice(b"{\"type\":\"hello\"}\n");
        let mut output: Vec<u8> = Vec::new();
        serve_async(&mut helper, &mut input.as_slice(), &mut output)
            .await
            .expect("serve");
        let stdout = String::from_utf8(output).expect("helper emits UTF-8");

        let events: Vec<HelperEvent> = stdout
            .lines()
            .map(|raw| serde_json::from_str(raw).expect("helper emits valid JSON"))
            .collect();
        assert_eq!(
            events,
            vec![
                HelperEvent::Hello {
                    helper_protocol: HELPER_PROTOCOL,
                    name: HELPER_NAME.into(),
                },
                HelperEvent::Error {
                    message: "malformed command".into(),
                },
            ],
            "blank and whitespace lines are skipped, and shutdown is terminal: \
             neither it nor any command after it produces output"
        );
    }

    #[tokio::test]
    async fn serve_stops_at_an_oversized_line_without_buffering_it() {
        let directory = tempfile::tempdir().unwrap();
        let mut helper = helper_with_store(directory.path());
        // Sized independently of MAX_LINE_READ so the assertion still means
        // something if that cap changes.
        let mut input: Vec<u8> = vec![b'x'; MAX_HELPER_LINE_BYTES + 4096];
        input.push(b'\n');
        input.extend_from_slice(b"{\"type\":\"hello\"}\n");
        let mut output: Vec<u8> = Vec::new();
        serve_async(&mut helper, &mut input.as_slice(), &mut output)
            .await
            .expect("serve");
        let stdout = String::from_utf8(output).expect("helper emits UTF-8");
        let events: Vec<HelperEvent> = stdout
            .lines()
            .map(|raw| serde_json::from_str(raw).expect("helper emits valid JSON"))
            .collect();
        assert_eq!(
            events,
            vec![HelperEvent::Error {
                message: "helper line too large".into(),
            }],
            "the read stops at the cap, so the oversized line is never echoed and \
             the command after it is never reached"
        );
        assert!(!stdout.contains('x'));
    }

    #[tokio::test]
    async fn serve_answers_a_final_command_that_has_no_trailing_newline() {
        let directory = tempfile::tempdir().unwrap();
        let mut helper = helper_with_store(directory.path());
        let input: Vec<u8> = b"{\"type\":\"hello\"}".to_vec();
        let mut output: Vec<u8> = Vec::new();
        serve_async(&mut helper, &mut input.as_slice(), &mut output)
            .await
            .expect("serve");
        assert_eq!(
            String::from_utf8(output)
                .expect("helper emits UTF-8")
                .lines()
                .map(|raw| serde_json::from_str::<HelperEvent>(raw).unwrap())
                .collect::<Vec<HelperEvent>>(),
            vec![HelperEvent::Hello {
                helper_protocol: HELPER_PROTOCOL,
                name: HELPER_NAME.into(),
            }]
        );
    }
    #[test]
    fn failed_session_discards_its_queued_reads_before_reconnect_selection() {
        let directory = tempfile::tempdir().unwrap();
        let mut helper = helper_with_store(directory.path());
        helper.online.insert("first".into());
        helper.pending_history = Some(HistoryRequest {
            account: "first".into(),
            conversation: "thread".into(),
            limit: 20,
            cursor: None,
            fetch_id: Some(7),
        });
        helper.pending_sync = Some("second".into());
        helper.end_live_session("first");
        assert!(!helper.online.contains("first"));
        assert!(
            helper.pending_history.is_none(),
            "the old request must not start another worker outside the retry policy"
        );
        assert_eq!(helper.pending_sync.as_deref(), Some("second"));
    }

    #[test]
    fn reconnect_is_bounded_and_never_retries_preemption_or_invalid_authentication() {
        use handover_google_messages::{ProbeError, receive::ReceiveError, session::SessionError};
        assert_eq!(
            reconnect_delay(&ProbeError::Network, 0).unwrap().as_secs(),
            1
        );
        assert_eq!(
            reconnect_delay(&ProbeError::ReceiveFailed, 4)
                .unwrap()
                .as_secs(),
            16
        );
        assert!(reconnect_delay(&ProbeError::Network, 5).is_none());
        for error in [
            ProbeError::HttpError(401),
            ProbeError::SessionExpired,
            ProbeError::ReceiveProtocol(ReceiveError::SessionPreempted),
            ProbeError::SessionProtocol(SessionError::UpdateAuthentication),
        ] {
            assert!(reconnect_delay(&error, 0).is_none());
        }
    }

    #[tokio::test]
    async fn live_messages_keep_thread_binding_without_closing_history() {
        use handover_core::messaging::{
            ConversationId, Message, MessageId, MessagingAccountId, Participant,
        };
        let records = (0..40)
            .map(|n| Message {
                id: MessageId::new(
                    ConversationId::new(
                        MessagingAccountId::new("fixture"),
                        if n % 2 == 0 { "first" } else { "second" },
                    ),
                    format!("m{n}"),
                ),
                sender: Participant {
                    local_id: "peer".into(),
                    display_name: None,
                    address: None,
                    is_self: false,
                },
                transport: None,
                sent_at: Some(n),
                text: Some("x".repeat(60_000)),
                attachments: vec![],
                reply_to: None,
                reactions: vec![],
                deleted: false,
            })
            .collect();
        let mut output = Vec::new();
        publish_push_messages(&mut output, "fixture".into(), records)
            .await
            .unwrap();
        let mut count = 0;
        let mut threads = BTreeSet::new();
        for raw in output
            .split(|byte| *byte == b'\n')
            .filter(|raw| !raw.is_empty())
        {
            assert!(raw.len() <= MAX_HELPER_LINE_BYTES);
            let HelperEvent::Messages {
                account,
                conversation,
                messages,
                cursor_next,
                page_complete,
                full,
                generation,
                fetch_id,
            } = serde_json::from_slice(raw).unwrap()
            else {
                panic!("messages");
            };
            assert_eq!(account, "fixture");
            assert!(!page_complete && !full);
            assert!(cursor_next.is_none() && generation.is_none() && fetch_id.is_none());
            assert!(messages.iter().all(|message| message.transport.is_none()));
            count += messages.len();
            threads.insert(conversation);
        }
        assert_eq!(count, 40);
        assert_eq!(threads.len(), 2);
    }

    #[tokio::test]
    async fn history_chunks_preserve_fetch_correlation_and_close_empty_pages() {
        use handover_core::messaging::{
            ConversationId, Message, MessageId, MessagingAccountId, Participant,
        };
        let conversation = ConversationId::new(MessagingAccountId::new("fixture"), "thread");
        let records = (0..30)
            .map(|index| Message {
                id: MessageId::new(conversation.clone(), format!("message-{index}")),
                sender: Participant {
                    local_id: "peer".into(),
                    display_name: None,
                    address: None,
                    is_self: false,
                },
                transport: None,
                sent_at: Some(1),
                text: Some("x".repeat(60_000)),
                attachments: Vec::new(),
                reply_to: None,
                reactions: Vec::new(),
                deleted: false,
            })
            .collect();
        let mut output = Vec::new();
        publish_history(
            &mut output,
            "fixture".into(),
            "thread".into(),
            Some(7),
            handover_google_messages::history::HistoryPage {
                messages: records,
                cursor_next: Some("opaque-cursor".into()),
            },
        )
        .await
        .unwrap();
        let lines: Vec<_> = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .collect();
        assert!(lines.len() > 1);
        let mut count = 0;
        for (index, line) in lines.iter().enumerate() {
            assert!(line.len() <= MAX_HELPER_LINE_BYTES);
            let HelperEvent::Messages {
                messages,
                fetch_id,
                page_complete,
                cursor_next,
                full,
                generation,
                ..
            } = serde_json::from_slice(line).unwrap()
            else {
                panic!("history event required")
            };
            count += messages.len();
            assert_eq!(fetch_id, Some(7));
            assert_eq!(page_complete, index + 1 == lines.len());
            assert!(!full);
            assert_eq!(generation, None);
            assert_eq!(
                cursor_next.as_deref(),
                if page_complete {
                    Some("opaque-cursor")
                } else {
                    None
                }
            );
        }
        assert_eq!(count, 30);
        output.clear();
        publish_history(
            &mut output,
            "fixture".into(),
            "thread".into(),
            Some(8),
            handover_google_messages::history::HistoryPage {
                messages: Vec::new(),
                cursor_next: None,
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            serde_json::from_slice::<HelperEvent>(&output).unwrap(),
            HelperEvent::Messages {
                page_complete: true,
                fetch_id: Some(8),
                ..
            }
        ));
    }
    #[test]
    fn cursor_links_evict_old_entries_without_crossing_account_or_thread() {
        use handover_core::messaging::{
            ConversationId, Message, MessageId, MessagingAccountId, Participant,
        };
        let directory = tempfile::tempdir().unwrap();
        let mut helper = helper_with_store(directory.path());
        for index in 0..=MAX_CURSOR_LINKS {
            let message = Message {
                id: MessageId::new(
                    ConversationId::new(MessagingAccountId::new("fixture"), "thread"),
                    format!("message-{index}"),
                ),
                sender: Participant {
                    local_id: "peer".into(),
                    display_name: None,
                    address: None,
                    is_self: false,
                },
                transport: None,
                sent_at: Some(index as i64),
                text: Some("Fixture".into()),
                attachments: Vec::new(),
                reply_to: None,
                reactions: Vec::new(),
                deleted: false,
            };
            helper.remember_cursor(
                "fixture",
                "thread",
                &handover_google_messages::history::HistoryPage {
                    messages: vec![message],
                    cursor_next: Some(format!("cursor-{index}")),
                },
            );
        }
        assert_eq!(helper.cursor_links.len(), MAX_CURSOR_LINKS);
        assert_eq!(helper.cursor_links.front().unwrap().message, "message-1");
        helper.save_cursor_links().unwrap();
        let record = helper
            .store
            .as_ref()
            .unwrap()
            .history_cursor_store()
            .unwrap()
            .load("index")
            .unwrap()
            .unwrap();
        assert!(record.as_bytes().len() < MAX_CURSOR_BYTES);
        assert!(
            !record
                .as_bytes()
                .windows(b"Fixture".len())
                .any(|bytes| bytes == b"Fixture")
        );
        let mut recovered = helper_with_store(directory.path());
        recovered.restore_cursor_links().unwrap();
        assert!(
            recovered.cursor_links.is_empty(),
            "unpaired accounts cannot recover cursor state"
        );
    }
}
