use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use handover_core::{
    Conversation, ConversationEvent, Message, MessageEvent, MessagingAccount,
    MessagingAccountEvent, MessagingEvent, ReadState, StateEvent,
};
use serde::{Deserialize, Serialize};

use crate::state::StateStore;

const CACHE_VERSION: u32 = 1;
/// Upper bound for the on-disk cache. Restores above this are
/// rejected rather than buffered.
const MAX_CACHE_BYTES: u64 = 32 * 1024 * 1024;

/// Opt-out for the on-disk messaging cache. When the environment
/// variable `HANDOVER_MESSAGING_CACHE` is set to `0`, the daemon
/// neither restores nor persists messaging state: restarts rebuild
/// every conversation and window from the helper.
pub(crate) fn disabled() -> bool {
    std::env::var_os("HANDOVER_MESSAGING_CACHE").is_some_and(|value| value == "0")
}

#[derive(Deserialize, Serialize)]
struct MessagingCache {
    version: u32,
    accounts: Vec<MessagingAccount>,
    conversations: Vec<Conversation>,
    messages: Vec<Message>,
    read_states: Vec<ReadState>,
}

fn cache_path() -> io::Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")));
    base.map(|base| base.join("handover/messaging-cache.json"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))
}

pub(crate) fn restore(state: &mut StateStore) -> Result<(), Box<dyn std::error::Error>> {
    if disabled() {
        return Ok(());
    }
    let path = cache_path()?;
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    // Never buffer an unbounded file: a corrupt or hostile cache
    // cannot balloon the daemon at startup.
    if !metadata.is_file() || metadata.len() > MAX_CACHE_BYTES {
        return Err(
            io::Error::new(io::ErrorKind::InvalidData, "messaging cache is not usable").into(),
        );
    }
    let bytes = fs::read(path)?;
    let cache: MessagingCache = serde_json::from_slice(&bytes)?;
    if cache.version != CACHE_VERSION {
        return Ok(());
    }
    for mut account in cache.accounts {
        // Connectivity and authentication are runtime attestations. Cached
        // records provide labels and identity only until the helper reconnects.
        account.connected = false;
        account.authenticated = false;
        state.apply(StateEvent::Messaging(MessagingEvent::Account(
            MessagingAccountEvent::Added(account),
        )));
    }
    for conversation in cache.conversations {
        state.apply(StateEvent::Messaging(MessagingEvent::Conversation(
            ConversationEvent::Added(conversation),
        )));
    }
    for message in cache.messages {
        state.apply(StateEvent::Messaging(MessagingEvent::Message(
            MessageEvent::Added(message),
        )));
    }
    for read_state in cache.read_states {
        state.apply(StateEvent::Messaging(MessagingEvent::Read(read_state)));
    }
    Ok(())
}

pub(crate) fn persist(state: &Arc<RwLock<StateStore>>) -> Result<(), Box<dyn std::error::Error>> {
    if disabled() {
        return Ok(());
    }
    // Coalesce bursts: a 100-message chunk would otherwise snapshot,
    // serialize, fsync, and rename the whole cache ~100 times while
    // helper ingestion waits. At most one persist starts per
    // interval; the remainder is scheduled once at the interval end,
    // and anything still dirty flushes on shutdown.
    static LAST: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
    static DIRTY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static SCHEDULED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let now = std::time::Instant::now();
    let due = {
        let mut last = LAST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match *last {
            Some(at) if now.duration_since(at) < PERSIST_INTERVAL => false,
            _ => {
                *last = Some(now);
                DIRTY.store(false, std::sync::atomic::Ordering::Relaxed);
                true
            }
        }
    };
    if !due {
        DIRTY.store(true, std::sync::atomic::Ordering::Relaxed);
        if !SCHEDULED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let state = Arc::clone(state);
                handle.spawn(async move {
                    tokio::time::sleep(PERSIST_INTERVAL).await;
                    SCHEDULED.store(false, std::sync::atomic::Ordering::Relaxed);
                    if DIRTY.swap(false, std::sync::atomic::Ordering::Relaxed) {
                        if let Err(error) = persist_async(&state).await {
                            tracing::warn!(%error, "could not persist messaging cache");
                        }
                    }
                });
            } else {
                SCHEDULED.store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
        return Ok(());
    }
    // Due: schedule the write off the event path. Awaiting
    // serialization and fsync here would stall helper ingestion.
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            let state = Arc::clone(state);
            handle.spawn(async move {
                if let Err(error) = persist_async(&state).await {
                    tracing::warn!(%error, "could not persist messaging cache");
                }
            });
            Ok(())
        }
        // No runtime (unit tests): persist inline.
        Err(_) => persist_now_legacy(state),
    }
}

/// Flush a coalesced persist, e.g. on shutdown. Awaits completion.
pub(crate) async fn flush(
    state: &Arc<RwLock<StateStore>>,
) -> Result<(), Box<dyn std::error::Error>> {
    if disabled() {
        return Ok(());
    }
    persist_async(state).await
}

/// Minimum spacing between cache persists.
const PERSIST_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Snapshot generation counters. persist_async snapshots under the
/// state lock and writes off the event path; completions publish
/// only when newer than everything already published, so concurrent
/// persists cannot rewind the file.
static SNAPSHOT_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static PUBLISHED_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Serialize and write one snapshot off the caller's thread.
/// A publish mutex serializes concurrent persists end to end, so an
/// older snapshot can never win a rename race against a newer one.
async fn persist_async(state: &Arc<RwLock<StateStore>>) -> Result<(), Box<dyn std::error::Error>> {
    static PUBLISH: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    let _publish = PUBLISH
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let generation = SNAPSHOT_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut cache = {
        let guard = state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        MessagingCache {
            version: CACHE_VERSION,
            accounts: guard.messaging().snapshot_accounts(),
            conversations: guard.messaging().snapshot_conversations(),
            messages: guard.messaging().snapshot_messages(),
            read_states: guard.messaging().snapshot_read(),
        }
    };
    prune_cache(&mut cache);
    let path = cache_path()?;
    let directory = path.parent().expect("cache path has parent").to_path_buf();
    fs::create_dir_all(&directory)?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    let encoded = tokio::task::spawn_blocking(move || {
        serde_json::to_vec(&cache)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    })
    .await
    .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)??;
    if PUBLISHED_GEN.fetch_max(generation, std::sync::atomic::Ordering::Relaxed) > generation {
        // A newer snapshot already published; writing this one would
        // rewind the file. Unreachable while publishes serialize,
        // kept as defense in depth.
        return Ok(());
    }
    // Unique temp file in the target directory: concurrent persists
    // must never share (and clobber) one temp path before the rename.
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
        temporary.as_file_mut().write_all(&encoded)?;
        temporary.as_file().sync_all()?;
        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o600))?;
        temporary.persist(&path).map_err(|error| {
            std::io::Error::new(
                error.error.kind(),
                format!("cache persist failed: {}", error.error),
            )
        })?;
        Ok(())
    })
    .await
    .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?
    .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?;
    Ok(())
}

/// Synchronous legacy persist for contexts without a runtime
/// (unit tests). Same atomic temp-file discipline as the async path.
fn persist_now_legacy(state: &Arc<RwLock<StateStore>>) -> Result<(), Box<dyn std::error::Error>> {
    if disabled() {
        return Ok(());
    }
    let mut cache = {
        let guard = state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        MessagingCache {
            version: CACHE_VERSION,
            accounts: guard.messaging().snapshot_accounts(),
            conversations: guard.messaging().snapshot_conversations(),
            messages: guard.messaging().snapshot_messages(),
            read_states: guard.messaging().snapshot_read(),
        }
    };
    prune_cache(&mut cache);
    let encoded = serde_json::to_vec(&cache)?;
    let path = cache_path()?;
    let directory = path.parent().expect("cache path has parent");
    fs::create_dir_all(directory)?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    // Unique temp file in the target directory: concurrent persists
    // must never share (and clobber) one temp path before the rename.
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.as_file_mut().write_all(&encoded)?;
    temporary.as_file().sync_all()?;
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o600))?;
    temporary.persist(&path)?;
    Ok(())
}

/// Drop oldest messages until the cache fits the restore bound, so
/// the daemon never writes a file it will refuse to read back.
/// Accounts, conversations, and read states are always kept whole.
fn prune_cache(cache: &mut MessagingCache) {
    let mut encoded_len = serde_json::to_vec(&cache)
        .map(|bytes| bytes.len())
        .unwrap_or(0);
    if encoded_len <= MAX_CACHE_BYTES as usize {
        return;
    }
    cache
        .messages
        .sort_by(|a, b| (a.sent_at, &a.id.local_id).cmp(&(b.sent_at, &b.id.local_id)));
    while encoded_len > MAX_CACHE_BYTES as usize && !cache.messages.is_empty() {
        let drop = (cache.messages.len() / 4).max(1);
        cache.messages.drain(..drop.min(cache.messages.len()));
        encoded_len = serde_json::to_vec(&cache)
            .map(|bytes| bytes.len())
            .unwrap_or(0);
    }
}
