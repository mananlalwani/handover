//! Helper-process backend for messaging.
//!
//! One optional supervisor task owns the helper child process. It is the
//! only place that touches helper I/O: it normalizes every helper record
//! before it becomes a [`StateEvent`], correlates request ids for commands
//! that need acceptance reports, and marks helper-owned accounts
//! disconnected when the helper dies. A dead helper never touches device,
//! notification, media, or share state.
//!
//! Logging rule: ids, counts, and delivery states only. Bodies, prompts,
//! and helper error text are never logged with content.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use handover_core::{
    ConversationEvent, ConversationId, MessageEvent, MessageId, MessageStatusUpdate,
    MessagingAccountEvent, MessagingAccountId, MessagingEvent, PairingPrompt, ReadState,
    TypingState,
};
use handover_gmessages::MAX_BUNDLE_BYTES;
use handover_gmessages::contract::{HelperCommand, HelperEvent, WireMessage};
use handover_gmessages::normalize::{
    event_ids, normalize_account, normalize_conversation, normalize_message, parse_status,
    resolve_sender,
};
use handover_gmessages::staging::{
    adapter_staging_directory, default_staging_directory, import_staged_path,
    imported_staging_directory,
};
use handover_gmessages::supervisor::{HelperProcess, backoff_delay, find_helper, redact_command};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tracing::{info, warn};

use crate::{apply_backend_event, publish_event, state::StateStore};

use handover_core::StateEvent;

// Google Messages relay operations can take about 60 seconds while the
// phone wakes and services a request. Keep this above the adapter's
// bounded slow-operation timeout so accepted commands are not reported
// as failures merely because the daemon stopped waiting early.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(4 * 60);
const MAX_IN_FLIGHT: usize = 64;
const DORMANT_RETRY: Duration = Duration::from_secs(30);
/// A helper session that survives this long counts as healthy and
/// resets the restart backoff. Shorter sessions keep counting up.
const STABLE_SESSION: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HelperCallError {
    Unavailable,
    Busy,
    Timeout,
    Rejected(String),
    BadBundle,
}

pub(crate) struct CommandOutcome {
    pub(crate) request_id: String,
}

#[derive(Clone)]
pub(crate) struct MessagingHub {
    inner: Arc<HubInner>,
}

type FetchWaiters = HashMap<(String, String), (u64, oneshot::Sender<Result<(), ()>>)>;

/// Open chunk generations: helper syncs split large lists and windows
/// into size-bounded chunks. Chunks that share a generation accumulate
/// here; the daemon reconciles only when the closing chunk arrives, so
/// threads and messages in later chunks are never briefly removed.
/// Ungrouped (single-chunk and live) events bypass these buffers.
type ConversationGenerations = HashMap<String, (u64, HashSet<String>)>;
type WindowGenerations = HashMap<(String, String), (u64, HashSet<String>)>;
/// One in-flight history page per conversation, serialized because
/// helper pages carry no request identity.
type FetchSerializers = HashMap<(String, String), FetchSerializer>;

struct FetchSerializer {
    lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    waiters: usize,
}
/// Oldest normalized message of the in-progress page, per conversation.
type PageFloors = HashMap<(String, String), (Option<i64>, String)>;

struct HubInner {
    sender: Mutex<Option<mpsc::Sender<HelperCommand>>>,
    pending: Mutex<HashMap<String, oneshot::Sender<Result<(), String>>>>,
    fetches: Mutex<FetchWaiters>,
    /// Per-conversation fetch serialization: helper pages carry no
    /// request identity, so concurrent fetches for one conversation
    /// with different cursors cannot be told apart. Waiters queue
    /// here instead of sharing one flight.
    fetch_locks: Mutex<FetchSerializers>,
    /// Waiters queued on fetch locks. Bounds total history waiters;
    /// `fetches` alone cannot, since one key holds one flight.
    fetch_waiters: std::sync::atomic::AtomicUsize,
    /// Whether the current helper echoes fetch ids. Learned from
    /// observed events; cleared whenever the helper disconnects so a
    /// replacement helper starts in legacy mode.
    fetch_id_supported: std::sync::atomic::AtomicBool,
    counter: AtomicU64,
    shutdown: AtomicBool,
    /// Account id -> (generation, announced conversation ids).
    conversation_syncs: Mutex<ConversationGenerations>,
    /// (Account id, conversation id) -> (generation, announced message ids).
    window_syncs: Mutex<WindowGenerations>,
    /// Oldest normalized message seen since the last emitted page
    /// cursor, per conversation. History pages arrive in chunks but
    /// only the closing chunk carries `cursor_next`; the public
    /// cursor must be derived from the complete page, not that chunk
    /// alone. Interleaved live messages also fold in; the floor is a
    /// minimum, so at worst a page overlaps, never gaps.
    page_floors: Mutex<PageFloors>,
}

/// Upper bound for one generation's keep-set. A malicious or broken
/// helper cannot grow these buffers without limit; overflowing
/// generations are dropped with a warning and fall back to
/// per-chunk handling.
const MAX_GENERATION_IDS: usize = 8192;

impl MessagingHub {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(HubInner {
                sender: Mutex::new(None),
                pending: Mutex::new(HashMap::new()),
                fetches: Mutex::new(HashMap::new()),
                counter: AtomicU64::new(1),
                shutdown: AtomicBool::new(false),
                fetch_locks: Mutex::new(HashMap::new()),
                fetch_waiters: std::sync::atomic::AtomicUsize::new(0),
                fetch_id_supported: std::sync::atomic::AtomicBool::new(false),
                page_floors: Mutex::new(HashMap::new()),
                conversation_syncs: Mutex::new(HashMap::new()),
                window_syncs: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Graceful shutdown: stop the supervisor loop and ask the helper to
    /// exit. Unclean kills can still orphan the helper; the next supervisor
    /// generation replaces it.
    pub(crate) async fn shutdown(&self) {
        self.inner.shutdown.store(true, Ordering::Relaxed);
        let _ = self.fire(HelperCommand::Shutdown).await;
    }

    pub(crate) fn is_shutdown(&self) -> bool {
        self.inner.shutdown.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn try_has_sender(&self) -> bool {
        self.inner
            .sender
            .try_lock()
            .map(|guard| guard.is_some())
            .unwrap_or(true)
    }

    fn next_request_id(&self) -> String {
        let id = self.inner.counter.fetch_add(1, Ordering::Relaxed);
        format!("msgreq-{id}")
    }

    async fn submit(&self, command: HelperCommand) -> Result<(), HelperCallError> {
        let sender = self.inner.sender.lock().await.clone();
        match sender {
            Some(sender) => sender
                .send(command)
                .await
                .map_err(|_| HelperCallError::Unavailable),
            None => Err(HelperCallError::Unavailable),
        }
    }

    /// Send a command that reports acceptance via `CommandResult`.
    pub(crate) async fn request(
        &self,
        build: impl FnOnce(String) -> HelperCommand,
    ) -> Result<CommandOutcome, HelperCallError> {
        let request_id = self.next_request_id();
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.inner.pending.lock().await;
            if pending.len() >= MAX_IN_FLIGHT {
                return Err(HelperCallError::Busy);
            }
            pending.insert(request_id.clone(), tx);
        }
        let command = build(request_id.clone());
        if let Err(error) = self.submit(command).await {
            self.inner.pending.lock().await.remove(&request_id);
            return Err(error);
        }
        match tokio::time::timeout(COMMAND_TIMEOUT, rx).await {
            Ok(Ok(Ok(()))) => Ok(CommandOutcome { request_id }),
            Ok(Ok(Err(reason))) => Err(HelperCallError::Rejected(reason)),
            Ok(Err(_)) => Err(HelperCallError::Unavailable),
            Err(_) => {
                self.inner.pending.lock().await.remove(&request_id);
                Err(HelperCallError::Timeout)
            }
        }
    }

    /// Fire-and-forget command (read receipts, typing pings, login/logout,
    /// list/sync). Acceptance means the daemon queued it for the helper.
    pub(crate) async fn fire(&self, command: HelperCommand) -> Result<(), HelperCallError> {
        self.submit(command).await
    }

    /// Page history through the helper and wait for the matching window.
    /// Fetches for one conversation run one at a time: helper pages
    /// carry no request identity, so concurrent cursors would all be
    /// completed by whichever page arrives first.
    pub(crate) async fn fetch_through_helper(
        &self,
        account: &str,
        conversation: &str,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(), HelperCallError> {
        let key = (account.to_string(), conversation.to_string());
        if self.inner.fetch_waiters.fetch_add(1, Ordering::Relaxed) >= MAX_IN_FLIGHT {
            self.inner.fetch_waiters.fetch_sub(1, Ordering::Relaxed);
            return Err(HelperCallError::Busy);
        }
        let result = self
            .fetch_serialized(&key, account, conversation, limit, cursor)
            .await;
        self.inner.fetch_waiters.fetch_sub(1, Ordering::Relaxed);
        result
    }

    async fn fetch_serialized(
        &self,
        key: &(String, String),
        account: &str,
        conversation: &str,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(), HelperCallError> {
        let serializer = {
            let mut locks = self.inner.fetch_locks.lock().await;
            let serializer = locks.entry(key.clone()).or_insert_with(|| FetchSerializer {
                lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
                waiters: 0,
            });
            serializer.waiters += 1;
            serializer.lock.clone()
        };
        let _guard = serializer.lock().await;
        let fetch_id = self.inner.counter.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            self.inner
                .fetches
                .lock()
                .await
                .insert(key.clone(), (fetch_id, tx));
        }
        if let Err(error) = self
            .submit(HelperCommand::FetchHistory {
                account: account.into(),
                conversation: conversation.into(),
                limit,
                cursor,
                fetch_id: Some(fetch_id),
            })
            .await
        {
            self.take_fetch(key, fetch_id).await;
            self.release_serializer(key).await;
            return Err(error);
        }
        let outcome = match tokio::time::timeout(COMMAND_TIMEOUT, rx).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(()))) | Ok(Err(_)) => Err(HelperCallError::Unavailable),
            Err(_) => {
                // Drop only our own waiter. Without this, the next
                // page_complete would consume a completion that
                // belongs to a later fetch.
                self.take_fetch(key, fetch_id).await;
                Err(HelperCallError::Timeout)
            }
        };
        self.release_serializer(key).await;
        outcome
    }

    /// Decrement one serializer's waiter count, removing the entry
    /// when the last waiter leaves so the map cannot grow with the
    /// number of conversations ever fetched.
    async fn release_serializer(&self, key: &(String, String)) {
        let mut locks = self.inner.fetch_locks.lock().await;
        if let Some(serializer) = locks.get_mut(key) {
            serializer.waiters = serializer.waiters.saturating_sub(1);
            if serializer.waiters == 0 {
                locks.remove(key);
            }
        }
    }

    /// Remove one fetch waiter, but only if it is still ours.
    async fn take_fetch(&self, key: &(String, String), fetch_id: u64) {
        let mut fetches = self.inner.fetches.lock().await;
        if fetches.get(key).is_some_and(|(id, _)| *id == fetch_id) {
            fetches.remove(key);
        }
    }

    async fn complete_request(&self, request_id: &str, result: Result<(), String>) {
        if let Some(sender) = self.inner.pending.lock().await.remove(request_id) {
            let _ = sender.send(result);
        }
    }

    async fn complete_fetch(&self, account: &str, conversation: &str, fetch_id: Option<u64>) {
        if fetch_id.is_some() {
            // Helpers that echo fetch ids make completion exact: only
            // the matching waiter wakes, so background windows can
            // never complete an explicit fetch early.
            self.inner.fetch_id_supported.store(true, Ordering::Relaxed);
        }
        let key = (account.to_string(), conversation.to_string());
        let mut fetches = self.inner.fetches.lock().await;
        let matched = match fetches.get(&key) {
            Some((expected, _)) => {
                if self.inner.fetch_id_supported.load(Ordering::Relaxed) {
                    fetch_id.is_some_and(|id| id == *expected)
                } else {
                    // Legacy helper: no echo to match on. Key-only
                    // completion, with its known cross-talk limits.
                    true
                }
            }
            None => false,
        };
        if matched {
            if let Some((_, waiter)) = fetches.remove(&key) {
                let _ = waiter.send(Ok(()));
            }
        }
    }

    async fn fail_fetches(&self) {
        for (_, (_, waiter)) in self.inner.fetches.lock().await.drain() {
            let _ = waiter.send(Err(()));
        }
    }

    async fn fail_all(&self) {
        for (_, sender) in self.inner.pending.lock().await.drain() {
            let _ = sender.send(Err("helper disconnected".into()));
        }
        for (_, (_, waiter)) in self.inner.fetches.lock().await.drain() {
            let _ = waiter.send(Err(()));
        }
        // A dead helper never closes its open generations. Drop them so
        // the next helper generation starts from clean buffers instead
        // of reconciling against a stale partial set. Fetch-id support
        // is relearned from the replacement helper.
        self.inner.conversation_syncs.lock().await.clear();
        self.inner.window_syncs.lock().await.clear();
        self.inner.page_floors.lock().await.clear();
        self.inner
            .fetch_id_supported
            .store(false, Ordering::Relaxed);
    }

    /// Accumulate one conversation chunk into its generation buffer.
    /// Returns `false` when the buffer overflowed and the caller must
    /// fall back to immediate handling for this chunk.
    async fn accumulate_conversations(
        &self,
        account: &str,
        generation: u64,
        ids: HashSet<String>,
    ) -> bool {
        let mut syncs = self.inner.conversation_syncs.lock().await;
        match syncs.get_mut(account) {
            Some((open, keep)) if *open == generation => {
                keep.extend(ids);
            }
            _ => {
                if syncs.contains_key(account) {
                    warn!("conversation generation changed mid-sync; restarting buffer");
                }
                syncs.insert(account.to_string(), (generation, ids));
            }
        }
        if syncs
            .get(account)
            .is_some_and(|(_, keep)| keep.len() > MAX_GENERATION_IDS)
        {
            warn!("conversation generation overflowed; falling back to per-chunk handling");
            syncs.remove(account);
            return false;
        }
        true
    }

    /// Close one conversation generation, returning the accumulated
    /// keep-set for a single reconcile. `None` means the generation
    /// was unknown or overflowed: reconcile against this chunk alone.
    async fn close_conversations(
        &self,
        account: &str,
        generation: u64,
        ids: HashSet<String>,
    ) -> Option<HashSet<String>> {
        let mut syncs = self.inner.conversation_syncs.lock().await;
        match syncs.remove(account) {
            Some((open, mut keep)) if open == generation => {
                keep.extend(ids);
                Some(keep)
            }
            _ => {
                warn!(
                    "conversation generation closed without matching open; reconciling chunk alone"
                );
                None
            }
        }
    }

    /// Fold a live (ungrouped) conversation into any open generation so
    /// the closing reconcile does not treat it as stale.
    async fn note_live_conversation(&self, account: &str, id: &str) {
        let mut syncs = self.inner.conversation_syncs.lock().await;
        if let Some((_, keep)) = syncs.get_mut(account) {
            keep.insert(id.to_string());
        }
    }

    /// Fold a live conversation removal into any open generation.
    async fn note_removed_conversation(&self, account: &str, id: &str) {
        let mut syncs = self.inner.conversation_syncs.lock().await;
        if let Some((_, keep)) = syncs.get_mut(account) {
            keep.remove(id);
        }
    }

    /// Accumulate one message-window chunk. Contract mirrors
    /// [`Self::accumulate_conversations`].
    async fn accumulate_window(
        &self,
        account: &str,
        conversation: &str,
        generation: u64,
        ids: HashSet<String>,
    ) -> bool {
        let key = (account.to_string(), conversation.to_string());
        let mut syncs = self.inner.window_syncs.lock().await;
        match syncs.get_mut(&key) {
            Some((open, keep)) if *open == generation => {
                keep.extend(ids);
            }
            _ => {
                if syncs.contains_key(&key) {
                    warn!("window generation changed mid-sync; restarting buffer");
                }
                syncs.insert(key.clone(), (generation, ids));
            }
        }
        if syncs
            .get(&key)
            .is_some_and(|(_, keep)| keep.len() > MAX_GENERATION_IDS)
        {
            warn!("window generation overflowed; falling back to per-chunk handling");
            syncs.remove(&key);
            return false;
        }
        true
    }

    /// Close one window generation. Contract mirrors
    /// [`Self::close_conversations`].
    async fn close_window(
        &self,
        account: &str,
        conversation: &str,
        generation: u64,
        ids: HashSet<String>,
    ) -> Option<HashSet<String>> {
        let key = (account.to_string(), conversation.to_string());
        let mut syncs = self.inner.window_syncs.lock().await;
        match syncs.remove(&key) {
            Some((open, mut keep)) if open == generation => {
                keep.extend(ids);
                Some(keep)
            }
            _ => {
                warn!("window generation closed without matching open; reconciling chunk alone");
                None
            }
        }
    }

    /// Fold a live (ungrouped) message into any open window generation.
    async fn note_live_message(&self, account: &str, conversation: &str, id: &str) {
        let key = (account.to_string(), conversation.to_string());
        let mut syncs = self.inner.window_syncs.lock().await;
        if let Some((_, keep)) = syncs.get_mut(&key) {
            keep.insert(id.to_string());
        }
    }

    /// Track the oldest normalized message of the in-progress page.
    async fn accumulate_page_floor(
        &self,
        account: &str,
        conversation: &str,
        messages: &[(Option<i64>, String)],
    ) {
        let key = (account.to_string(), conversation.to_string());
        let mut floors = self.inner.page_floors.lock().await;
        for (sent_at, id) in messages {
            let candidate = (*sent_at, id.clone());
            match floors.get_mut(&key) {
                Some(floor) => {
                    if candidate < *floor {
                        *floor = candidate;
                    }
                }
                None => {
                    floors.insert(key.clone(), candidate);
                }
            }
        }
    }

    /// Take the page's oldest message id and reset the floor. Called
    /// when the closing chunk's cursor arrives, or when a page ends
    /// without one.
    async fn take_page_floor(&self, account: &str, conversation: &str) -> Option<String> {
        self.inner
            .page_floors
            .lock()
            .await
            .remove(&(account.to_string(), conversation.to_string()))
            .map(|(_, id)| id)
    }

    /// The fetch id an outstanding history fetch expects, if any.
    async fn expected_fetch(&self, account: &str, conversation: &str) -> Option<u64> {
        self.inner
            .fetches
            .lock()
            .await
            .get(&(account.to_string(), conversation.to_string()))
            .map(|(id, _)| *id)
    }

    /// Whether any fetch id has ever been echoed back. Gates the
    /// legacy key-only matching used with helpers that predate ids.
    async fn fetch_id_known(&self) -> bool {
        self.inner.fetch_id_supported.load(Ordering::Relaxed)
    }

    async fn set_sender(&self, sender: Option<mpsc::Sender<HelperCommand>>) {
        *self.inner.sender.lock().await = sender;
    }
}

pub(crate) fn spawn_supervisor(
    state: Arc<std::sync::RwLock<StateStore>>,
    events: broadcast::Sender<StateEvent>,
    hub: MessagingHub,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut restarts: u32 = 0;
        loop {
            if hub.is_shutdown() {
                break;
            }
            let Some(path) = find_helper() else {
                tokio::time::sleep(DORMANT_RETRY).await;
                continue;
            };
            match HelperProcess::spawn(&path).await {
                Ok(process) => {
                    info!(helper = %process.name, "messaging helper connected");
                    let started = std::time::Instant::now();
                    run_session(&state, &events, &hub, process).await;
                    hub.fail_all().await;
                    if hub.is_shutdown() {
                        break;
                    }
                    mark_helper_accounts_down(&state, &events);
                    // Only a session that stayed up earns a reset. A
                    // helper that handshakes and then crashes repeatedly
                    // must back off progressively instead of respawning
                    // every couple of seconds forever.
                    if started.elapsed() >= STABLE_SESSION {
                        restarts = 0;
                    }
                }
                Err(error) => {
                    warn!(%error, "messaging helper unavailable; retrying");
                }
            }
            restarts += 1;
            tokio::time::sleep(backoff_delay(restarts)).await;
        }
    })
}

async fn run_session(
    state: &Arc<std::sync::RwLock<StateStore>>,
    events: &broadcast::Sender<StateEvent>,
    hub: &MessagingHub,
    mut process: HelperProcess,
) {
    let (tx, mut rx) = mpsc::channel::<HelperCommand>(128);
    hub.set_sender(Some(tx)).await;
    // Catch-up: ask the helper to re-emit authoritative state for every
    // account we know. Unknown accounts announce themselves via Account.
    let known: Vec<MessagingAccountId> = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .messaging()
        .snapshot_accounts()
        .into_iter()
        .map(|account| account.id)
        .collect();
    let mut seen: HashSet<String> = HashSet::new();
    // Catch-up: ask the helper to re-emit authoritative state for every
    // account we know before serving new events.
    for account in &known {
        seen.insert(account.as_str().to_string());
        let _ = hub
            .fire(HelperCommand::Sync {
                account: account.as_str().into(),
            })
            .await;
    }
    // Outbound pump and inbound reader share one mutable borrow of the
    // helper process inside a single select loop.
    loop {
        tokio::select! {
            command = rx.recv() => {
                match command {
                    Some(command) => {
                        // Login bundles are redacted; everything else is ids.
                        tracing::debug!(command = %redact_command(&command), "helper command");
                        if process.send(&command).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            event = process.next_event() => {
                match event {
                    Ok(Some(event)) => {
                        tracing::debug!(event = %event_ids(&event), "helper event");
                        ingest_event(state, events, hub, &mut seen, event).await;
                    }
                    Ok(None) => break,
                    Err(error) => {
                        warn!(%error, "dropping malformed helper message");
                    }
                }
            }
        }
    }
    hub.set_sender(None).await;
}

async fn ingest_event(
    state: &Arc<std::sync::RwLock<StateStore>>,
    events: &broadcast::Sender<StateEvent>,
    hub: &MessagingHub,
    seen: &mut HashSet<String>,
    event: HelperEvent,
) {
    match event {
        HelperEvent::Hello { .. } => {}
        HelperEvent::Account {
            account,
            label,
            connected,
            authenticated,
        } => {
            seen.insert(account.clone());
            match normalize_account(&account, &label, connected, authenticated) {
                Ok(record) => {
                    let id = record.id.clone();
                    let known = state
                        .read()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .messaging()
                        .account(&id)
                        .is_some();
                    let change = if known {
                        MessagingAccountEvent::Updated(record)
                    } else {
                        MessagingAccountEvent::Added(record)
                    };
                    apply_backend_event(
                        state,
                        events,
                        StateEvent::Messaging(MessagingEvent::Account(change)),
                    );
                    if !known {
                        // Fresh account (e.g. just logged in): ask the helper
                        // for its authoritative state now; session-start
                        // catch-up only covered previously known accounts.
                        let _ = hub
                            .fire(HelperCommand::Sync {
                                account: id.as_str().into(),
                            })
                            .await;
                    }
                }
                Err(error) => warn!(%error, "dropping invalid account record"),
            }
        }
        HelperEvent::AccountRemoved { account } => {
            apply_backend_event(
                state,
                events,
                StateEvent::Messaging(MessagingEvent::Account(MessagingAccountEvent::Removed(
                    MessagingAccountId::new(account),
                ))),
            );
        }
        HelperEvent::Pairing { account, prompt } => {
            if prompt.len() > 512 {
                warn!("dropping oversized pairing prompt");
                return;
            }
            apply_backend_event(
                state,
                events,
                StateEvent::Messaging(MessagingEvent::Pairing(PairingPrompt {
                    account_id: MessagingAccountId::new(account),
                    prompt,
                })),
            );
        }
        HelperEvent::Conversations {
            account,
            conversations,
            full,
            generation,
        } => {
            let account_id = MessagingAccountId::new(account.clone());
            let mut announced = HashSet::new();
            for wire in conversations {
                match normalize_conversation(&account_id, wire) {
                    Ok(record) => {
                        let id = record.id.clone();
                        announced.insert(id.local_id.clone());
                        let known = state
                            .read()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .messaging()
                            .conversation(&id)
                            .is_some();
                        let change = if known {
                            ConversationEvent::Updated(record)
                        } else {
                            ConversationEvent::Added(record)
                        };
                        hub.note_live_conversation(&account, &id.local_id).await;
                        apply_backend_event(
                            state,
                            events,
                            StateEvent::Messaging(MessagingEvent::Conversation(change)),
                        );
                    }
                    Err(error) => warn!(%error, "dropping invalid conversation record"),
                }
            }
            // Reconcile removals only against a complete set. Intermediate
            // generation chunks merge; reconciling them would briefly
            // remove threads that arrive in later chunks and destroy
            // their cached messages, reads, and statuses.
            match (generation, full) {
                (Some(generation), false) => {
                    if !hub
                        .accumulate_conversations(&account, generation, announced.clone())
                        .await
                    {
                        reconcile_against(state, events, &account_id, &announced);
                    }
                }
                (Some(generation), true) => {
                    match hub
                        .close_conversations(&account, generation, announced.clone())
                        .await
                    {
                        Some(keep) => {
                            reconcile_against(state, events, &account_id, &keep);
                        }
                        None => {
                            reconcile_against(state, events, &account_id, &announced);
                        }
                    }
                }
                (None, true) => {
                    reconcile_against(state, events, &account_id, &announced);
                }
                (None, false) => {}
            }
        }
        HelperEvent::ConversationRemoved {
            account,
            conversation,
        } => {
            hub.note_removed_conversation(&account, &conversation).await;
            apply_backend_event(
                state,
                events,
                StateEvent::Messaging(MessagingEvent::Conversation(ConversationEvent::Removed(
                    ConversationId::new(MessagingAccountId::new(account), conversation),
                ))),
            );
        }
        HelperEvent::Messages {
            account,
            conversation,
            messages,
            full,
            cursor_next,
            page_complete,
            generation,
            fetch_id,
        } => {
            let conversation_id = ConversationId::new(
                MessagingAccountId::new(account.clone()),
                conversation.clone(),
            );
            // Helper cursors contain backend-private relay details. Public
            // paging uses the oldest normalized message id; the helper can
            // recover its private timestamp from its own cache.
            let mut normalized = Vec::with_capacity(messages.len());
            for wire in messages {
                let wire = scrub_staged_paths(wire);
                match normalize_message(&conversation_id, wire) {
                    Ok(mut message) => {
                        if let Some(conversation) = state
                            .read()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .messaging()
                            .conversation(&conversation_id)
                            .cloned()
                        {
                            resolve_sender(&mut message, &conversation);
                        }
                        normalized.push(message);
                    }
                    Err(error) => warn!(%error, "dropping invalid message record"),
                }
            }
            // The public cursor comes from the complete accumulated page:
            // only the closing chunk carries `cursor_next`, but older
            // chunks hold older messages. Accumulation is scoped to
            // real pages: an open generation, a page boundary, or an
            // outstanding fetch whose id matches this event. Ordinary
            // live traffic must not move it, and neither may one
            // fetch's pages contaminate another's.
            let expected = hub.expected_fetch(&account, &conversation).await;
            let fetch_matches = match (expected, fetch_id) {
                (Some(expected), Some(actual)) => expected == actual,
                // Legacy helpers never echo an id: fall back to key
                // matching while none has ever been observed.
                (Some(_), None) => !hub.fetch_id_known().await,
                _ => false,
            };
            let is_page = generation.is_some()
                || cursor_next
                    .as_deref()
                    .is_some_and(|cursor| !cursor.is_empty())
                || page_complete
                || fetch_matches;
            if is_page {
                let floor_points: Vec<(Option<i64>, String)> = normalized
                    .iter()
                    .map(|message| (message.sent_at, message.id.local_id.clone()))
                    .collect();
                hub.accumulate_page_floor(&account, &conversation, &floor_points)
                    .await;
            }
            let public_cursor = match cursor_next.filter(|cursor| !cursor.is_empty()) {
                Some(_) => hub.take_page_floor(&account, &conversation).await,
                None => {
                    if page_complete {
                        hub.take_page_floor(&account, &conversation).await;
                    }
                    None
                }
            };
            let conversation_record = {
                state
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .messaging()
                    .conversation(&conversation_id)
                    .cloned()
            };
            if let Some(mut record) = conversation_record {
                // Only page events move the history cursor. A live
                // message carries no cursor_next and must not clear
                // the older-history position readers depend on.
                if is_page {
                    record.cursor = public_cursor;
                    apply_backend_event(
                        state,
                        events,
                        StateEvent::Messaging(MessagingEvent::Conversation(
                            ConversationEvent::Updated(record),
                        )),
                    );
                }
            }
            // Windows reconcile removals only against a complete set, for
            // the same reason as conversation lists: reconciling an
            // intermediate chunk would drop messages that arrive later.
            match (generation, full) {
                (None, true) => {
                    let outcome = state
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .messaging_mut()
                        .reconcile_window(&conversation_id, normalized);
                    publish_outcome(state, events, outcome);
                    if let Err(error) = crate::messaging_cache::persist(state) {
                        warn!(%error, "could not persist messaging cache");
                    }
                }
                (Some(generation), true) => {
                    let announced: HashSet<String> = normalized
                        .iter()
                        .map(|message| message.id.local_id.clone())
                        .collect();
                    merge_messages(state, events, &conversation_id, normalized);
                    for id in &announced {
                        hub.note_live_message(&account, &conversation, id).await;
                    }
                    match hub
                        .close_window(&account, &conversation, generation, announced.clone())
                        .await
                    {
                        Some(keep) => {
                            let keep: std::collections::BTreeSet<String> =
                                keep.into_iter().collect();
                            let outcome = state
                                .write()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .messaging_mut()
                                .prune_window(&conversation_id, &keep);
                            publish_outcome(state, events, outcome);
                            if let Err(error) = crate::messaging_cache::persist(state) {
                                warn!(%error, "could not persist messaging cache");
                            }
                        }
                        None => {
                            // No matching open generation: this chunk is
                            // the only authoritative set available.
                            let keep: std::collections::BTreeSet<String> =
                                announced.into_iter().collect();
                            let outcome = state
                                .write()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .messaging_mut()
                                .prune_window(&conversation_id, &keep);
                            publish_outcome(state, events, outcome);
                            if let Err(error) = crate::messaging_cache::persist(state) {
                                warn!(%error, "could not persist messaging cache");
                            }
                        }
                    }
                }
                (Some(generation), false) => {
                    let announced: HashSet<String> = normalized
                        .iter()
                        .map(|message| message.id.local_id.clone())
                        .collect();
                    merge_messages(state, events, &conversation_id, normalized);
                    if !hub
                        .accumulate_window(&account, &conversation, generation, announced)
                        .await
                    {
                        state
                            .write()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .messaging_mut()
                            .sort_window(&conversation_id);
                    }
                }
                (None, false) => {
                    for message in &normalized {
                        hub.note_live_message(&account, &conversation, &message.id.local_id)
                            .await;
                    }
                    merge_messages(state, events, &conversation_id, normalized);
                    state
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .messaging_mut()
                        .sort_window(&conversation_id);
                }
            }
            if page_complete {
                hub.complete_fetch(&account, &conversation, fetch_id).await;
            }
        }
        HelperEvent::MessageRemoved {
            account,
            conversation,
            message,
        } => {
            apply_backend_event(
                state,
                events,
                StateEvent::Messaging(MessagingEvent::Message(MessageEvent::Removed(
                    MessageId::new(
                        ConversationId::new(MessagingAccountId::new(account), conversation),
                        message,
                    ),
                ))),
            );
        }
        HelperEvent::Status {
            account,
            conversation,
            message,
            status,
        } => match parse_status(&status) {
            Ok(parsed) => {
                apply_backend_event(
                    state,
                    events,
                    StateEvent::Messaging(MessagingEvent::Status(MessageStatusUpdate {
                        message_id: MessageId::new(
                            ConversationId::new(MessagingAccountId::new(account), conversation),
                            message,
                        ),
                        status: parsed,
                    })),
                );
            }
            Err(error) => warn!(%error, "dropping unknown message status"),
        },
        HelperEvent::Typing {
            account,
            conversation,
            participants,
        } => {
            if participants.len() > 256 {
                warn!("dropping oversized typing update");
                return;
            }
            apply_backend_event(
                state,
                events,
                StateEvent::Messaging(MessagingEvent::Typing(TypingState {
                    conversation_id: ConversationId::new(
                        MessagingAccountId::new(account),
                        conversation,
                    ),
                    participant_ids: participants,
                })),
            );
        }
        HelperEvent::Read {
            account,
            conversation,
            last_read_message,
            unread,
        } => {
            let conversation_id =
                ConversationId::new(MessagingAccountId::new(account), conversation);
            apply_backend_event(
                state,
                events,
                StateEvent::Messaging(MessagingEvent::Read(ReadState {
                    conversation_id: conversation_id.clone(),
                    last_read_message_id: last_read_message
                        .map(|local_id| MessageId::new(conversation_id, local_id)),
                    unread,
                })),
            );
        }
        HelperEvent::CommandResult {
            request_id,
            ok,
            error,
        } => {
            hub.complete_request(
                &request_id,
                ok.then_some(()).ok_or_else(|| error.unwrap_or_default()),
            )
            .await;
        }
        HelperEvent::Error { message } => {
            // Helper text may contain credentials or message content. Keep
            // only stable metadata in the journal.
            warn!(error_len = message.len(), "messaging helper error");
            hub.fail_fetches().await;
        }
    }
}

/// Validate helper-reported staged paths before they enter daemon
/// state. Unverifiable paths are dropped; attachment metadata stays so the
/// client still sees that an attachment exists.
fn scrub_staged_paths(mut wire: WireMessage) -> WireMessage {
    let roots = [default_staging_directory(), adapter_staging_directory()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let import_directory = imported_staging_directory().ok();
    for attachment in &mut wire.attachments {
        if let Some(path) = attachment.staged_path.take() {
            let imported = import_directory
                .as_deref()
                .and_then(|directory| import_staged_path(&path, &roots, directory).ok());
            match imported {
                Some(valid) => attachment.staged_path = valid.to_str().map(str::to_string),
                None => warn!("dropping unverifiable staged attachment path"),
            }
        }
    }
    wire
}

/// Merge normalized messages without reconciling removals: upsert
/// each record as added or updated. Used for live events and for
/// generation chunks whose removals wait for the closing chunk.
fn merge_messages(
    state: &Arc<std::sync::RwLock<StateStore>>,
    events: &broadcast::Sender<StateEvent>,
    conversation_id: &ConversationId,
    messages: Vec<handover_core::Message>,
) {
    for message in messages {
        let known = state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .messaging()
            .message(&message.id)
            .is_some();
        let change = if known {
            MessageEvent::Updated(message)
        } else {
            MessageEvent::Added(message)
        };
        apply_backend_event(
            state,
            events,
            StateEvent::Messaging(MessagingEvent::Message(change)),
        );
    }
    state
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .messaging_mut()
        .sort_window(conversation_id);
}

/// Remove stored conversations for one account that are absent from a
/// complete keep-set (one full list or one closed generation).
fn reconcile_against(
    state: &Arc<std::sync::RwLock<StateStore>>,
    events: &broadcast::Sender<StateEvent>,
    account_id: &MessagingAccountId,
    keep: &HashSet<String>,
) {
    let stale: Vec<ConversationId> = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .messaging()
        .snapshot_conversations()
        .into_iter()
        .map(|conversation| conversation.id)
        .filter(|id| &id.account_id == account_id && !keep.contains(&id.local_id))
        .collect();
    for id in stale {
        apply_backend_event(
            state,
            events,
            StateEvent::Messaging(MessagingEvent::Conversation(ConversationEvent::Removed(id))),
        );
    }
}

/// Publish an already-applied reconcile outcome: translate each stored
/// change back into its event form so subscribers observe the same
/// authoritative window the daemon now holds.
fn publish_outcome(
    state: &Arc<std::sync::RwLock<StateStore>>,
    events: &broadcast::Sender<StateEvent>,
    outcome: crate::messaging::MessagingOutcome,
) {
    use crate::messaging::MessagingChange;
    for change in outcome.changes {
        crate::log_messaging_change(change.clone());
        let event = match &change {
            MessagingChange::MessageAdded(id) | MessagingChange::MessageUpdated(id) => {
                let guard = state
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match guard.messaging().message(id) {
                    Some(message) => {
                        let event = if matches!(change, MessagingChange::MessageAdded(_)) {
                            MessageEvent::Added(message.clone())
                        } else {
                            MessageEvent::Updated(message.clone())
                        };
                        Some(StateEvent::Messaging(MessagingEvent::Message(event)))
                    }
                    None => None,
                }
            }
            MessagingChange::MessageRemoved(id) => Some(StateEvent::Messaging(
                MessagingEvent::Message(MessageEvent::Removed(id.clone())),
            )),
            _ => None,
        };
        if let Some(event) = event {
            publish_event(events, event);
        }
    }
}

fn mark_helper_accounts_down(
    state: &Arc<std::sync::RwLock<StateStore>>,
    events: &broadcast::Sender<StateEvent>,
) {
    let accounts: Vec<handover_core::MessagingAccount> = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .messaging()
        .snapshot_accounts();
    for mut account in accounts {
        if !account.connected {
            continue;
        }
        account.connected = false;
        apply_backend_event(
            state,
            events,
            StateEvent::Messaging(MessagingEvent::Account(MessagingAccountEvent::Updated(
                account,
            ))),
        );
    }
    info!("messaging helper disconnected; accounts marked offline");
}

pub(crate) fn validate_login_bundle(bundle_b64: &str) -> Result<(), HelperCallError> {
    if bundle_b64.is_empty() || bundle_b64.len() > MAX_BUNDLE_BYTES {
        return Err(HelperCallError::BadBundle);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use handover_gmessages::contract::{
        WireConversation, WireConversationKind, WireMessage, WireParticipant, WireTransport,
    };

    fn wire_conversation(id: &str) -> WireConversation {
        WireConversation {
            local_id: id.into(),
            kind: WireConversationKind::Direct,
            transport: WireTransport::Rcs,
            title: None,
            participants: vec![WireParticipant {
                local_id: "other".into(),
                display_name: None,
                address: Some("+15550000000".into()),
                is_self: false,
            }],
            latest_message: None,
            last_activity_at: None,
            unread_count: None,
            cursor: None,
            capabilities: vec!["text".into()],
        }
    }

    fn wire_message(id: &str) -> WireMessage {
        WireMessage {
            local_id: id.into(),
            sender: "other".into(),
            transport: Some(WireTransport::Rcs),
            sent_at: Some(1),
            text: Some("message".into()),
            attachments: Vec::new(),
            reply_to: None,
            reactions: Vec::new(),
            deleted: false,
        }
    }

    fn conversation_ids(state: &Arc<std::sync::RwLock<StateStore>>) -> Vec<String> {
        let guard = state.read().unwrap();
        let mut ids: Vec<String> = guard
            .messaging()
            .snapshot_conversations()
            .into_iter()
            .map(|conversation| conversation.id.local_id)
            .collect();
        ids.sort();
        ids
    }

    fn window_ids(
        state: &Arc<std::sync::RwLock<StateStore>>,
        conversation: &ConversationId,
    ) -> Vec<String> {
        let guard = state.read().unwrap();
        let mut ids: Vec<String> = guard
            .messaging()
            .snapshot_messages()
            .into_iter()
            .filter(|message| message.id.conversation_id == *conversation)
            .map(|message| message.id.local_id)
            .collect();
        ids.sort();
        ids
    }

    #[tokio::test]
    async fn conversation_generation_reconciles_once_on_close() {
        let state = Arc::new(std::sync::RwLock::new(StateStore::default()));
        let (events, _) = broadcast::channel(64);
        let hub = MessagingHub::new();
        let mut seen = HashSet::new();
        let account = "personal".to_string();
        ingest_event(
            &state,
            &events,
            &hub,
            &mut seen,
            HelperEvent::Account {
                account: account.clone(),
                label: "Messages".into(),
                connected: true,
                authenticated: true,
            },
        )
        .await;

        // Seed one thread with a legacy full list.
        ingest_event(
            &state,
            &events,
            &hub,
            &mut seen,
            HelperEvent::Conversations {
                account: account.clone(),
                conversations: vec![wire_conversation("seeded")],
                full: true,
                generation: None,
            },
        )
        .await;
        assert_eq!(conversation_ids(&state), vec!["seeded"]);

        // Intermediate chunk merges: nothing is removed yet, even
        // though "seeded" is absent from this chunk.
        ingest_event(
            &state,
            &events,
            &hub,
            &mut seen,
            HelperEvent::Conversations {
                account: account.clone(),
                conversations: vec![wire_conversation("t1")],
                full: false,
                generation: Some(9),
            },
        )
        .await;
        assert_eq!(conversation_ids(&state), vec!["seeded", "t1"]);

        // A live merge during the open generation joins the keep-set.
        ingest_event(
            &state,
            &events,
            &hub,
            &mut seen,
            HelperEvent::Conversations {
                account: account.clone(),
                conversations: vec![wire_conversation("live")],
                full: false,
                generation: None,
            },
        )
        .await;

        // Closing chunk reconciles once against the whole generation:
        // "seeded" drops (absent everywhere), the rest stay.
        ingest_event(
            &state,
            &events,
            &hub,
            &mut seen,
            HelperEvent::Conversations {
                account: account.clone(),
                conversations: vec![wire_conversation("t2")],
                full: true,
                generation: Some(9),
            },
        )
        .await;
        assert_eq!(conversation_ids(&state), vec!["live", "t1", "t2"]);
    }

    #[tokio::test]
    async fn window_generation_prunes_once_on_close() {
        let state = Arc::new(std::sync::RwLock::new(StateStore::default()));
        let (events, _) = broadcast::channel(64);
        let hub = MessagingHub::new();
        let mut seen = HashSet::new();
        let conversation =
            ConversationId::new(MessagingAccountId::new("personal"), "thread".to_string());
        ingest_event(
            &state,
            &events,
            &hub,
            &mut seen,
            HelperEvent::Account {
                account: "personal".into(),
                label: "Messages".into(),
                connected: true,
                authenticated: true,
            },
        )
        .await;

        ingest_event(
            &state,
            &events,
            &hub,
            &mut seen,
            HelperEvent::Conversations {
                account: "personal".into(),
                conversations: vec![wire_conversation("thread")],
                full: true,
                generation: None,
            },
        )
        .await;
        ingest_event(
            &state,
            &events,
            &hub,
            &mut seen,
            HelperEvent::Messages {
                account: "personal".into(),
                conversation: "thread".into(),
                messages: vec![wire_message("m1"), wire_message("m2")],
                cursor_next: None,
                page_complete: false,
                full: true,
                fetch_id: None,
                generation: None,
            },
        )
        .await;
        assert_eq!(window_ids(&state, &conversation), vec!["m1", "m2"]);

        // Intermediate chunk merges without pruning the stored window.
        ingest_event(
            &state,
            &events,
            &hub,
            &mut seen,
            HelperEvent::Messages {
                account: "personal".into(),
                conversation: "thread".into(),
                messages: vec![wire_message("m3")],
                cursor_next: None,
                page_complete: false,
                full: false,
                fetch_id: None,
                generation: Some(4),
            },
        )
        .await;
        assert_eq!(window_ids(&state, &conversation), vec!["m1", "m2", "m3"]);

        // Closing chunk prunes once against the whole generation.
        ingest_event(
            &state,
            &events,
            &hub,
            &mut seen,
            HelperEvent::Messages {
                account: "personal".into(),
                conversation: "thread".into(),
                messages: vec![wire_message("m4")],
                cursor_next: None,
                page_complete: true,
                full: true,
                fetch_id: None,
                generation: Some(4),
            },
        )
        .await;
        assert_eq!(window_ids(&state, &conversation), vec!["m3", "m4"]);
    }

    #[tokio::test]
    async fn fetch_completion_reaches_only_its_waiter() {
        let hub = MessagingHub::new();
        let key = ("a".to_string(), "c".to_string());
        let (tx, rx) = oneshot::channel();
        hub.inner.fetches.lock().await.insert(key.clone(), (7, tx));
        // A stale timeout must not consume a newer waiter's completion.
        hub.take_fetch(&key, 6).await;
        assert!(hub.inner.fetches.lock().await.contains_key(&key));
        hub.complete_fetch("a", "c", Some(7)).await;
        assert!(rx.await.expect("waiter completes").is_ok());
        assert!(!hub.inner.fetches.lock().await.contains_key(&key));
    }

    #[tokio::test]
    async fn mismatched_fetch_id_never_completes_a_waiter() {
        let hub = MessagingHub::new();
        let key = ("a".to_string(), "c".to_string());
        let (tx, rx) = oneshot::channel();
        hub.inner
            .fetch_id_supported
            .store(true, std::sync::atomic::Ordering::Relaxed);
        hub.inner.fetches.lock().await.insert(key.clone(), (7, tx));
        // A background window (no id) must not complete the waiter
        // once ids are in play.
        hub.complete_fetch("a", "c", None).await;
        assert!(hub.inner.fetches.lock().await.contains_key(&key));
        // Neither may a different fetch's page.
        hub.complete_fetch("a", "c", Some(8)).await;
        assert!(hub.inner.fetches.lock().await.contains_key(&key));
        hub.complete_fetch("a", "c", Some(7)).await;
        assert!(rx.await.expect("waiter completes").is_ok());
    }

    #[tokio::test]
    async fn page_cursor_comes_from_the_whole_page() {
        let state = Arc::new(std::sync::RwLock::new(StateStore::default()));
        let (events, _) = broadcast::channel(64);
        let hub = MessagingHub::new();
        let mut seen = HashSet::new();
        let conversation =
            ConversationId::new(MessagingAccountId::new("personal"), "thread".to_string());
        let rate = |id: &str, sent_at: i64| WireMessage {
            local_id: id.into(),
            sender: "other".into(),
            transport: Some(WireTransport::Rcs),
            sent_at: Some(sent_at),
            text: Some("message".into()),
            attachments: Vec::new(),
            reply_to: None,
            reactions: Vec::new(),
            deleted: false,
        };
        for event in [
            HelperEvent::Account {
                account: "personal".into(),
                label: "Messages".into(),
                connected: true,
                authenticated: true,
            },
            HelperEvent::Conversations {
                account: "personal".into(),
                conversations: vec![wire_conversation("thread")],
                full: true,
                generation: None,
            },
            // Newer chunk first: the cursor must still come from the
            // older chunk that arrives last.
            HelperEvent::Messages {
                account: "personal".into(),
                conversation: "thread".into(),
                messages: vec![rate("m5", 5)],
                cursor_next: None,
                page_complete: false,
                full: false,
                fetch_id: None,
                generation: None,
            },
            HelperEvent::Messages {
                account: "personal".into(),
                conversation: "thread".into(),
                messages: vec![rate("m3", 3)],
                cursor_next: Some("relay:9".into()),
                page_complete: true,
                full: false,
                fetch_id: None,
                generation: None,
            },
        ] {
            ingest_event(&state, &events, &hub, &mut seen, event).await;
        }
        let cursor = state
            .read()
            .unwrap()
            .messaging()
            .conversation(&conversation)
            .and_then(|conversation| conversation.cursor.clone());
        assert_eq!(cursor.as_deref(), Some("m3"));
    }

    #[tokio::test]
    async fn failed_send_for_accepted_transaction_surfaces() {
        let state = Arc::new(std::sync::RwLock::new(StateStore::default()));
        let (events, mut receiver) = broadcast::channel(64);
        let hub = MessagingHub::new();
        let mut seen = HashSet::new();
        ingest_event(
            &state,
            &events,
            &hub,
            &mut seen,
            HelperEvent::Account {
                account: "personal".into(),
                label: "Messages".into(),
                connected: true,
                authenticated: true,
            },
        )
        .await;
        // Drain the account announcement; only status broadcasts matter below.
        while receiver.try_recv().is_ok() {}
        let transaction = MessageId::new(
            ConversationId::new(MessagingAccountId::new("personal"), "thread".to_string()),
            "txn-1".to_string(),
        );

        // Acceptance for an unknown transaction opens correlation
        // without broadcasting.
        ingest_event(
            &state,
            &events,
            &hub,
            &mut seen,
            HelperEvent::Status {
                account: "personal".into(),
                conversation: "thread".into(),
                message: "txn-1".into(),
                status: "accepted".into(),
            },
        )
        .await;
        assert!(receiver.try_recv().is_err());

        // The later transport failure is broadcast once instead of
        // vanishing with the unknown message.
        ingest_event(
            &state,
            &events,
            &hub,
            &mut seen,
            HelperEvent::Status {
                account: "personal".into(),
                conversation: "thread".into(),
                message: "txn-1".into(),
                status: "failed:transport".into(),
            },
        )
        .await;
        match receiver.try_recv() {
            Ok(StateEvent::Messaging(MessagingEvent::Status(update))) => {
                assert_eq!(update.message_id, transaction);
            }
            other => panic!("expected a status broadcast, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn message_page_cursor_update_does_not_hold_state_read_lock() {
        let state = Arc::new(std::sync::RwLock::new(StateStore::default()));
        let (events, _) = broadcast::channel(8);
        let hub = MessagingHub::new();
        let mut seen = HashSet::new();

        ingest_event(
            &state,
            &events,
            &hub,
            &mut seen,
            HelperEvent::Account {
                account: "personal".into(),
                label: "Messages".into(),
                connected: true,
                authenticated: true,
            },
        )
        .await;
        ingest_event(
            &state,
            &events,
            &hub,
            &mut seen,
            HelperEvent::Conversations {
                account: "personal".into(),
                conversations: vec![WireConversation {
                    local_id: "thread".into(),
                    kind: WireConversationKind::Direct,
                    transport: WireTransport::Rcs,
                    title: None,
                    participants: vec![WireParticipant {
                        local_id: "other".into(),
                        display_name: None,
                        address: Some("+15550000000".into()),
                        is_self: false,
                    }],
                    latest_message: None,
                    last_activity_at: None,
                    unread_count: None,
                    cursor: None,
                    capabilities: vec!["text".into()],
                }],
                full: true,
                generation: None,
            },
        )
        .await;

        tokio::time::timeout(
            Duration::from_secs(1),
            ingest_event(
                &state,
                &events,
                &hub,
                &mut seen,
                HelperEvent::Messages {
                    account: "personal".into(),
                    conversation: "thread".into(),
                    messages: vec![WireMessage {
                        local_id: "oldest".into(),
                        sender: "other".into(),
                        transport: Some(WireTransport::Rcs),
                        sent_at: Some(1),
                        text: Some("message".into()),
                        attachments: Vec::new(),
                        reply_to: None,
                        reactions: Vec::new(),
                        deleted: false,
                    }],
                    cursor_next: Some("older:123".into()),
                    page_complete: true,
                    full: false,
                    fetch_id: None,
                    generation: None,
                },
            ),
        )
        .await
        .expect("cursor update must not deadlock");

        let id = ConversationId::new(MessagingAccountId::new("personal"), "thread");
        let cursor = state
            .read()
            .unwrap()
            .messaging()
            .conversation(&id)
            .and_then(|conversation| conversation.cursor.as_deref())
            .map(str::to_owned);
        assert_eq!(cursor.as_deref(), Some("oldest"));
    }
}
