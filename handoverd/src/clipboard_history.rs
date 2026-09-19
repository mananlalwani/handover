use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use handover_ipc::ClipboardHistoryEntry;
use serde::{Deserialize, Serialize};

const HISTORY_VERSION: u32 = 1;
const RECENT_LIMIT: usize = 25;
const PINNED_LIMIT: usize = 25;
const TEXT_LIMIT: usize = 32 * 1024;
// Leave room below handover-ipc's 1 MiB line limit for the response envelope.
const STORED_HISTORY_LIMIT: usize = 900 * 1024;

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
            .and_then(read_bounded)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<StoredHistory>(&bytes).ok())
            .filter(|stored| stored.version == HISTORY_VERSION)
            .map(recover)
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
        validate_limits(&candidate)?;
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
        validate_limits(&candidate)?;
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

fn recover(mut stored: StoredHistory) -> StoredHistory {
    let mut ids = BTreeSet::new();
    let mut texts = BTreeSet::new();
    stored.entries.retain(|entry| {
        !entry.text.is_empty()
            && entry.text.len() <= TEXT_LIMIT
            && ids.insert(entry.id)
            && texts.insert(entry.text.clone())
    });
    trim(&mut stored.entries);
    let mut pinned = 0;
    stored.entries.retain(|entry| {
        if !entry.pinned {
            return true;
        }
        pinned += 1;
        pinned <= PINNED_LIMIT
    });
    while validate_limits(&stored).is_err() && !stored.entries.is_empty() {
        stored.entries.pop();
    }
    stored.next_id = stored
        .entries
        .iter()
        .map(|entry| entry.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
        .max(1);
    stored
}

fn read_bounded(path: PathBuf) -> io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take((STORED_HISTORY_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > STORED_HISTORY_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "clipboard history file is too large",
        ));
    }
    Ok(bytes)
}

fn validate_limits(state: &StoredHistory) -> io::Result<()> {
    if state.entries.iter().filter(|entry| entry.pinned).count() > PINNED_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "clipboard history has too many pinned entries",
        ));
    }
    if serde_json::to_vec(state)?.len() > STORED_HISTORY_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "clipboard history is full",
        ));
    }
    Ok(())
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

    #[test]
    fn recovery_bounds_pinned_entries() {
        let entries: Vec<_> = (0..30)
            .map(|id| ClipboardHistoryEntry {
                id,
                text: id.to_string(),
                pinned: true,
                created_at_ms: id,
            })
            .collect();
        let recovered = recover(StoredHistory {
            version: HISTORY_VERSION,
            next_id: 31,
            entries,
        });
        assert_eq!(recovered.entries.len(), PINNED_LIMIT);
        assert_eq!(recovered.entries.last().map(|entry| entry.id), Some(24));
    }

    #[test]
    fn recovery_drops_invalid_duplicate_and_excess_entries() {
        let mut entries: Vec<_> = (0..30)
            .map(|id| ClipboardHistoryEntry {
                id,
                text: format!("entry-{id}"),
                pinned: true,
                created_at_ms: id,
            })
            .collect();
        entries.push(ClipboardHistoryEntry {
            id: 0,
            text: "duplicate-id".into(),
            pinned: false,
            created_at_ms: 31,
        });
        entries.push(ClipboardHistoryEntry {
            id: 99,
            text: String::new(),
            pinned: false,
            created_at_ms: 32,
        });
        let recovered = recover(StoredHistory {
            version: HISTORY_VERSION,
            next_id: 1,
            entries,
        });
        assert_eq!(recovered.entries.len(), PINNED_LIMIT);
        assert_eq!(recovered.next_id, 25);
    }

    #[test]
    fn bounded_reader_rejects_oversized_files() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("history.json");
        fs::write(&path, vec![b'x'; STORED_HISTORY_LIMIT + 1]).expect("write history");
        let error = read_bounded(path).expect_err("oversized history must fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn limits_reject_too_many_pinned_entries() {
        let entries = (0..=PINNED_LIMIT)
            .map(|id| ClipboardHistoryEntry {
                id: id as u64,
                text: id.to_string(),
                pinned: true,
                created_at_ms: id as u64,
            })
            .collect();
        let error = validate_limits(&StoredHistory {
            version: HISTORY_VERSION,
            next_id: PINNED_LIMIT as u64 + 1,
            entries,
        })
        .expect_err("excess pinned entries must fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}
