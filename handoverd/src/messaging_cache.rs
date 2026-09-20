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
    let path = cache_path()?;
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
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
    let cache = {
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
