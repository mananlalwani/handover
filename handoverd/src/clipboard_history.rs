use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use handover_ipc::ClipboardHistoryEntry;
use serde::{Deserialize, Serialize};

const HISTORY_VERSION: u32 = 1;
const RECENT_LIMIT: usize = 25;
const TEXT_LIMIT: usize = 32 * 1024;

#[derive(Clone, Default, Deserialize, Serialize)]
struct StoredHistory {
    version: u32,
    next_id: u64,
    entries: Vec<ClipboardHistoryEntry>,
}

pub(crate) struct ClipboardHistory {
    state: Mutex<StoredHistory>,
}

impl ClipboardHistory {
    pub(crate) fn load() -> Self {
        let state = history_path()
            .and_then(fs::read)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<StoredHistory>(&bytes).ok())
            .filter(|stored| stored.version == HISTORY_VERSION)
            .unwrap_or_else(|| StoredHistory {
                version: HISTORY_VERSION,
                next_id: 1,
                entries: Vec::new(),
            });
        Self {
            state: Mutex::new(state),
        }
    }

    pub(crate) fn entries(&self) -> Vec<ClipboardHistoryEntry> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .clone()
    }

    pub(crate) fn record_phone_text(&self, text: &str) -> io::Result<()> {
        self.record(text, false)
    }

    pub(crate) fn save_pinned(&self, text: &str) -> io::Result<()> {
        self.record(text, true)
    }

    fn record(&self, text: &str, pinned: bool) -> io::Result<()> {
        if text.is_empty() || text.len() > TEXT_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "clipboard text must contain 1 to 32768 bytes",
            ));
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut candidate = state.clone();
        if let Some(index) = candidate
            .entries
            .iter()
            .position(|entry| entry.text == text)
        {
            let mut entry = candidate.entries.remove(index);
            entry.pinned |= pinned;
            entry.created_at_ms = now_ms();
            candidate.entries.insert(0, entry);
        } else {
            let id = candidate.next_id;
            candidate.next_id = candidate.next_id.saturating_add(1);
            candidate.entries.insert(
                0,
                ClipboardHistoryEntry {
                    id,
                    text: text.to_owned(),
                    pinned,
                    created_at_ms: now_ms(),
                },
            );
        }
        trim(&mut candidate.entries);
        persist(&candidate)?;
        *state = candidate;
        Ok(())
    }

    pub(crate) fn set_pinned(&self, id: u64, pinned: bool) -> io::Result<bool> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut candidate = state.clone();
        let Some(entry) = candidate.entries.iter_mut().find(|entry| entry.id == id) else {
            return Ok(false);
        };
        entry.pinned = pinned;
        trim(&mut candidate.entries);
        persist(&candidate)?;
        *state = candidate;
        Ok(true)
    }

    pub(crate) fn text(&self, id: u64) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.text.clone())
    }

    pub(crate) fn clear(&self, include_pinned: bool) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut candidate = state.clone();
        if include_pinned {
            candidate.entries.clear();
        } else {
            candidate.entries.retain(|entry| entry.pinned);
        }
        persist(&candidate)?;
        *state = candidate;
        Ok(())
    }
}

fn trim(entries: &mut Vec<ClipboardHistoryEntry>) {
    let mut recent = 0;
    entries.retain(|entry| {
        if entry.pinned {
            true
        } else {
            recent += 1;
            recent <= RECENT_LIMIT
        }
    });
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn history_path() -> io::Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")));
    base.map(|base| base.join("handover/clipboard-history.json"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))
}

fn persist(state: &StoredHistory) -> io::Result<()> {
    let encoded = serde_json::to_vec(state)?;
    let path = history_path()?;
    let directory = path.parent().expect("history path has parent");
    fs::create_dir_all(directory)?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    let temporary = path.with_extension("json.tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(&encoded)?;
    file.sync_all()?;
    fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_keeps_twenty_five_recent_entries_and_all_pins() {
        let mut entries: Vec<_> = (0..30)
            .map(|id| ClipboardHistoryEntry {
                id,
                text: id.to_string(),
                pinned: id == 29,
                created_at_ms: id,
            })
            .collect();
        trim(&mut entries);
        assert_eq!(entries.iter().filter(|entry| !entry.pinned).count(), 25);
        assert!(entries.iter().any(|entry| entry.id == 29));
    }
}
