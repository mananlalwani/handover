//! Small, private persistence for the messaging view and unsent drafts.
use handover_core::{
    ConversationId, MessageAttachmentDraft, MessageDraft, MessageId, MessagingLocation,
    MessagingWorkspace, validate_conversation_id as validate_core_conversation_id,
    validate_message_draft, validate_messaging_location,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::BTreeMap,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use thiserror::Error;

mod attachment_drafts;
mod draft_sends;

const MAX_DRAFTS: usize = 128;
const MAX_DRAFT_BYTES: usize = 64 * 1024;
const DATABASE_LIMIT_BYTES: i64 = 16 * 1024 * 1024;
type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Error)]
pub(crate) enum WorkspaceError {
    #[error("draft revision conflict")]
    Conflict,
    #[error("draft is invalid: {0}")]
    Invalid(String),
    #[error("workspace storage error: {0}")]
    Storage(String),
}

enum Store {
    Database(Connection),
    Memory(MemoryState),
}

#[derive(Default)]
struct MemoryState {
    location: Option<MessagingLocation>,
    drafts: BTreeMap<ConversationId, MessageDraft>,
    attachments: BTreeMap<ConversationId, MessageAttachmentDraft>,
    revision: u64,
}

pub(crate) struct Workspace {
    store: Mutex<Store>,
    persistent: bool,
    files: Option<attachment_drafts::FileRoot>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            store: Mutex::new(Store::Memory(MemoryState::default())),
            persistent: false,
            files: attachment_drafts::FileRoot::temporary(),
        }
    }
}

impl Workspace {
    pub(crate) fn open_default() -> Result<Arc<Self>, Error> {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
            .ok_or("state directory unavailable")?;
        Self::open(&base.join("handover/messaging-workspace.sqlite"))
    }

    pub(crate) fn open(path: &Path) -> Result<Arc<Self>, Error> {
        let parent = path.parent().ok_or("workspace parent unavailable")?;
        std::fs::create_dir_all(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        let db = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.pragma_update(None, "journal_mode", "DELETE")?;
        let page_size: i64 = db.pragma_query_value(None, "page_size", |row| row.get(0))?;
        let page_count: i64 = db.pragma_query_value(None, "page_count", |row| row.get(0))?;
        if page_count.saturating_mul(page_size) > DATABASE_LIMIT_BYTES {
            return Err("messaging workspace database exceeds 16 MiB".into());
        }
        db.pragma_update(None, "max_page_count", DATABASE_LIMIT_BYTES / page_size)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS workspace_meta(key TEXT PRIMARY KEY, value INTEGER NOT NULL);
             INSERT OR IGNORE INTO workspace_meta(key,value) VALUES('revision',0);
             CREATE TABLE IF NOT EXISTS drafts(account TEXT NOT NULL, thread TEXT NOT NULL,
                text TEXT NOT NULL, reply_to TEXT, revision INTEGER NOT NULL,
                PRIMARY KEY(account,thread));
             CREATE TABLE IF NOT EXISTS location(id INTEGER PRIMARY KEY CHECK(id=1),
                account TEXT NOT NULL, thread TEXT NOT NULL, message TEXT);
             CREATE TABLE IF NOT EXISTS attachment_drafts(account TEXT NOT NULL,thread TEXT NOT NULL,
                payload TEXT NOT NULL,revision INTEGER NOT NULL,PRIMARY KEY(account,thread));",
        )?;
        let has_pending = db
            .prepare("PRAGMA table_info(drafts)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .any(|name| name == "pending_operation");
        if !has_pending {
            db.execute("ALTER TABLE drafts ADD COLUMN pending_operation TEXT", [])?;
        }
        let workspace = Arc::new(Self {
            store: Mutex::new(Store::Database(db)),
            persistent: true,
            files: Some(attachment_drafts::FileRoot::persistent(parent)?),
        });
        workspace.clean_attachment_files()?;
        Ok(workspace)
    }

    pub(crate) async fn get(self: &Arc<Self>) -> Result<MessagingWorkspace, Error> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.with_read(read_workspace)).await?
    }

    pub(crate) async fn select(
        self: &Arc<Self>,
        location: Option<MessagingLocation>,
    ) -> Result<MessagingWorkspace, Error> {
        if let Some(location) = &location {
            validate_location(location).map_err(|error| -> Error { error.into() })?;
        }
        let this = self.clone();
        let persistent = self.persistent;
        tokio::task::spawn_blocking(move || this.with_write(|store| {
            match store {
                Store::Memory(state) => state.location = location,
                Store::Database(db) => match &location {
                    Some(value) => { db.execute("INSERT INTO location VALUES(1,?1,?2,?3) ON CONFLICT(id) DO UPDATE SET account=excluded.account,thread=excluded.thread,message=excluded.message", params![value.conversation_id.account_id.as_str(), value.conversation_id.local_id, value.message_id.as_ref().map(|id| id.local_id.as_str())])?; }
                    None => { db.execute("DELETE FROM location WHERE id=1", [])?; }
                },
            }
            read_workspace(store, persistent)
        })).await?
    }

    pub(crate) async fn draft(self: &Arc<Self>, id: ConversationId) -> Result<MessageDraft, Error> {
        validate_conversation_id(&id).map_err(|error| -> Error { error.into() })?;
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            this.with_read(|store, _| match store {
                Store::Memory(state) => Ok(state
                    .drafts
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| empty_draft(id))),
                Store::Database(db) => load_draft(db, &id),
            })
        })
        .await?
    }

    pub(crate) async fn save(
        self: &Arc<Self>,
        draft: MessageDraft,
    ) -> Result<MessageDraft, WorkspaceError> {
        validate_draft(&draft)?;
        if draft.pending_operation.is_some() {
            return Err(WorkspaceError::Conflict);
        }
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.save_blocking(draft))
            .await
            .map_err(|error| WorkspaceError::Storage(error.to_string()))?
    }

    pub(crate) async fn remove_account(self: &Arc<Self>, account: String) -> Result<(), Error> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            this.with_write(|store| match store {
                Store::Memory(state) => {
                    state
                        .drafts
                        .retain(|id, _| id.account_id.as_str() != account);
                    state
                        .attachments
                        .retain(|id, _| id.account_id.as_str() != account);
                    if state.location.as_ref().is_some_and(|location| {
                        location.conversation_id.account_id.as_str() == account
                    }) {
                        state.location = None;
                    }
                    Ok(())
                }
                Store::Database(db) => {
                    let tx = db.transaction()?;
                    tx.execute("DELETE FROM drafts WHERE account=?1", [&account])?;
                    tx.execute("DELETE FROM attachment_drafts WHERE account=?1", [&account])?;
                    tx.execute("DELETE FROM location WHERE account=?1", [&account])?;
                    tx.commit()?;
                    Ok(())
                }
            })?;
            this.clean_attachment_files()?;
            Ok(())
        })
        .await?
    }

    fn save_blocking(&self, draft: MessageDraft) -> Result<MessageDraft, WorkspaceError> {
        let mut guard = self
            .store
            .lock()
            .map_err(|error| WorkspaceError::Storage(error.to_string()))?;
        match &mut *guard {
            Store::Memory(state) => {
                let existing = state
                    .drafts
                    .get(&draft.conversation_id)
                    .map_or(0, |value| value.revision);
                if state
                    .drafts
                    .get(&draft.conversation_id)
                    .is_some_and(|draft| draft.pending_operation.is_some())
                    || existing != draft.revision
                {
                    return Err(WorkspaceError::Conflict);
                }
                if draft.text.is_empty() && draft.reply_to.is_none() {
                    state.drafts.remove(&draft.conversation_id);
                    return Ok(empty_draft(draft.conversation_id));
                }
                if !state.drafts.contains_key(&draft.conversation_id)
                    && state.drafts.len() >= MAX_DRAFTS
                {
                    return Err(WorkspaceError::Invalid("draft limit reached".into()));
                }
                state.revision = state
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| WorkspaceError::Storage("revision exhausted".into()))?;
                let saved = MessageDraft {
                    revision: state.revision,
                    ..draft
                };
                state
                    .drafts
                    .insert(saved.conversation_id.clone(), saved.clone());
                Ok(saved)
            }
            Store::Database(db) => {
                let tx = db.transaction().map_err(storage)?;
                let current = load_draft_tx(&tx, &draft.conversation_id).map_err(storage)?;
                if current.pending_operation.is_some() || current.revision != draft.revision {
                    return Err(WorkspaceError::Conflict);
                }
                if draft.text.is_empty() && draft.reply_to.is_none() {
                    tx.execute(
                        "DELETE FROM drafts WHERE account=?1 AND thread=?2",
                        params![
                            draft.conversation_id.account_id.as_str(),
                            draft.conversation_id.local_id
                        ],
                    )
                    .map_err(storage)?;
                    tx.commit().map_err(storage)?;
                    return Ok(empty_draft(draft.conversation_id));
                }
                let count: i64 = tx
                    .query_row("SELECT count(*) FROM drafts", [], |row| row.get(0))
                    .map_err(storage)?;
                if current.revision == 0 && count >= MAX_DRAFTS as i64 {
                    return Err(WorkspaceError::Invalid("draft limit reached".into()));
                }
                let current_revision: i64 = tx
                    .query_row(
                        "SELECT value FROM workspace_meta WHERE key='revision'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(storage)?;
                let revision = current_revision
                    .checked_add(1)
                    .ok_or_else(|| WorkspaceError::Storage("revision exhausted".into()))?;
                tx.execute(
                    "UPDATE workspace_meta SET value=?1 WHERE key='revision'",
                    [revision],
                )
                .map_err(storage)?;
                let saved = MessageDraft {
                    revision: revision as u64,
                    ..draft
                };
                tx.execute("INSERT INTO drafts(account,thread,text,reply_to,revision) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(account,thread) DO UPDATE SET text=excluded.text,reply_to=excluded.reply_to,revision=excluded.revision", params![saved.conversation_id.account_id.as_str(), saved.conversation_id.local_id, saved.text, saved.reply_to.as_ref().map(|id| id.local_id.as_str()), revision]).map_err(storage)?;
                tx.commit().map_err(storage)?;
                Ok(saved)
            }
        }
    }

    fn with_read<T>(
        &self,
        f: impl FnOnce(&mut Store, bool) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let mut store = self.store.lock().map_err(|_| "workspace lock poisoned")?;
        f(&mut store, self.persistent)
    }

    fn with_write<T>(&self, f: impl FnOnce(&mut Store) -> Result<T, Error>) -> Result<T, Error> {
        let mut store = self.store.lock().map_err(|_| "workspace lock poisoned")?;
        // Make the storage mutex own all synchronous SQLite work. Callers run this on the blocking pool.
        f(&mut store)
    }
}

fn storage(error: impl std::fmt::Display) -> WorkspaceError {
    WorkspaceError::Storage(error.to_string())
}
fn empty_draft(id: ConversationId) -> MessageDraft {
    MessageDraft {
        conversation_id: id,
        text: String::new(),
        reply_to: None,
        revision: 0,
        pending_operation: None,
    }
}

fn validate_conversation_id(id: &ConversationId) -> Result<(), WorkspaceError> {
    validate_core_conversation_id(id).map_err(|error| WorkspaceError::Invalid(error.to_string()))
}

fn validate_location(location: &MessagingLocation) -> Result<(), WorkspaceError> {
    validate_messaging_location(location)
        .map_err(|error| WorkspaceError::Invalid(error.to_string()))
}

fn validate_draft(draft: &MessageDraft) -> Result<(), WorkspaceError> {
    validate_message_draft(draft).map_err(|error| WorkspaceError::Invalid(error.to_string()))?;
    if draft.text.len() > MAX_DRAFT_BYTES {
        return Err(WorkspaceError::Invalid("draft exceeds 64 KiB".into()));
    }
    if draft.text.chars().any(char::is_control)
        && draft
            .text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(WorkspaceError::Invalid(
            "draft contains a disallowed control character".into(),
        ));
    }
    Ok(())
}

fn read_workspace(store: &mut Store, persistent: bool) -> Result<MessagingWorkspace, Error> {
    let (location, drafts) = match store {
        Store::Memory(state) => (
            state.location.clone(),
            state
                .drafts
                .keys()
                .chain(
                    state
                        .attachments
                        .iter()
                        .filter(|(_, draft)| draft.attachment.is_some())
                        .map(|(id, _)| id),
                )
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect(),
        ),
        Store::Database(db) => {
            let location = db
                .query_row(
                    "SELECT account,thread,message FROM location WHERE id=1",
                    [],
                    |row| {
                        let account: String = row.get(0)?;
                        let thread: String = row.get(1)?;
                        let message: Option<String> = row.get(2)?;
                        let conversation_id = ConversationId::new(
                            handover_core::MessagingAccountId::new(account),
                            thread,
                        );
                        Ok(MessagingLocation {
                            conversation_id: conversation_id.clone(),
                            message_id: message
                                .map(|local_id| MessageId::new(conversation_id, local_id)),
                        })
                    },
                )
                .optional()?;
            let mut stmt =
                db.prepare("SELECT account,thread FROM drafts UNION SELECT account,thread FROM attachment_drafts WHERE json_extract(payload,'$.attachment') IS NOT NULL ORDER BY account,thread")?;
            let drafts = stmt
                .query_map([], |row| {
                    Ok(ConversationId::new(
                        handover_core::MessagingAccountId::new(row.get::<_, String>(0)?),
                        row.get::<_, String>(1)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            (location, drafts)
        }
    };
    Ok(MessagingWorkspace {
        location,
        draft_conversations: drafts,
        persistent,
    })
}

fn load_draft(db: &Connection, id: &ConversationId) -> Result<MessageDraft, Error> {
    load_draft_query(db, id)
}

fn load_draft_tx(
    tx: &rusqlite::Transaction<'_>,
    id: &ConversationId,
) -> Result<MessageDraft, Error> {
    load_draft_query(tx, id)
}

fn load_draft_query(db: &Connection, id: &ConversationId) -> Result<MessageDraft, Error> {
    Ok(db
        .query_row(
            "SELECT text,reply_to,revision,pending_operation FROM drafts WHERE account=?1 AND thread=?2",
            params![id.account_id.as_str(), id.local_id],
            |row| {
                let text: String = row.get(0)?;
                let reply: Option<String> = row.get(1)?;
                let revision: i64 = row.get(2)?;
                Ok(MessageDraft {
                    conversation_id: id.clone(),
                    text,
                    reply_to: reply.map(|local_id| MessageId::new(id.clone(), local_id)),
                    revision: revision as u64,
                    pending_operation: row.get(3)?,
                })
            },
        )
        .optional()?
        .unwrap_or_else(|| empty_draft(id.clone())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use handover_core::MessagingAccountId;

    fn id(thread: &str) -> ConversationId {
        ConversationId::new(MessagingAccountId::new("fixture"), thread)
    }
    fn draft(id: ConversationId, text: &str, revision: u64) -> MessageDraft {
        MessageDraft {
            conversation_id: id,
            text: text.into(),
            reply_to: None,
            revision,
            pending_operation: None,
        }
    }

    #[tokio::test]
    async fn draft_revision_survives_restart_and_clear_does_not_reuse_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.sqlite");
        let ws = Workspace::open(&path).unwrap();
        let saved = ws.save(draft(id("thread"), "first", 0)).await.unwrap();
        assert_eq!(ws.draft(id("thread")).await.unwrap(), saved);
        drop(ws);
        let ws = Workspace::open(&path).unwrap();
        assert_eq!(ws.draft(id("thread")).await.unwrap(), saved);
        let cleared = ws
            .save(draft(id("thread"), "", saved.revision))
            .await
            .unwrap();
        assert_eq!(cleared.revision, 0);
        let newer = ws.save(draft(id("thread"), "second", 0)).await.unwrap();
        assert!(newer.revision > saved.revision);
    }

    #[tokio::test]
    async fn concurrent_compare_and_swap_allows_only_one_writer() {
        let ws = Arc::new(Workspace::default());
        let (a, b) = tokio::join!(
            ws.save(draft(id("thread"), "a", 0)),
            ws.save(draft(id("thread"), "b", 0))
        );
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        assert!(matches!(
            if a.is_err() { a } else { b },
            Err(WorkspaceError::Conflict)
        ));
    }

    #[tokio::test]
    async fn account_removal_clears_its_drafts_and_location() {
        let ws = Arc::new(Workspace::default());
        let a = id("thread");
        let b = ConversationId::new(MessagingAccountId::new("other"), "thread");
        ws.save(draft(a.clone(), "draft", 0)).await.unwrap();
        ws.save(draft(b.clone(), "keep", 0)).await.unwrap();
        ws.select(Some(MessagingLocation {
            conversation_id: a.clone(),
            message_id: None,
        }))
        .await
        .unwrap();
        ws.remove_account("fixture".into()).await.unwrap();
        assert_eq!(ws.draft(a).await.unwrap().revision, 0);
        assert_eq!(ws.draft(b).await.unwrap().text, "keep");
        assert!(ws.get().await.unwrap().location.is_none());
    }

    #[tokio::test]
    async fn draft_limits_reject_without_overwriting_existing_value() {
        let ws = Arc::new(Workspace::default());
        assert!(matches!(
            ws.save(draft(
                id("thread"),
                "x".repeat(MAX_DRAFT_BYTES + 1).as_str(),
                0
            ))
            .await,
            Err(WorkspaceError::Invalid(_))
        ));
        for index in 0..MAX_DRAFTS {
            ws.save(draft(id(&format!("thread-{index}")), "x", 0))
                .await
                .unwrap();
        }
        assert!(matches!(
            ws.save(draft(id("overflow"), "x", 0)).await,
            Err(WorkspaceError::Invalid(_))
        ));
        assert_eq!(ws.draft(id("thread-0")).await.unwrap().text, "x");
    }

    #[tokio::test]
    async fn storage_failure_rolls_back_draft_and_revision() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::open(&dir.path().join("workspace.sqlite")).unwrap();
        let original = ws.save(draft(id("thread"), "kept", 0)).await.unwrap();
        {
            let mut store = ws.store.lock().unwrap();
            let Store::Database(db) = &mut *store else {
                panic!("persistent workspace should use SQLite");
            };
            db.execute_batch(
                "CREATE TRIGGER reject_draft_update BEFORE UPDATE ON drafts
                 BEGIN SELECT RAISE(ABORT, 'forced storage failure'); END;",
            )
            .unwrap();
        }

        assert!(matches!(
            ws.save(draft(id("thread"), "replacement", original.revision))
                .await,
            Err(WorkspaceError::Storage(_))
        ));
        assert_eq!(ws.draft(id("thread")).await.unwrap(), original);

        {
            let mut store = ws.store.lock().unwrap();
            let Store::Database(db) = &mut *store else {
                panic!("persistent workspace should use SQLite");
            };
            db.execute_batch("DROP TRIGGER reject_draft_update;")
                .unwrap();
        }
        let saved = ws
            .save(draft(id("thread"), "replacement", original.revision))
            .await
            .unwrap();
        assert_eq!(saved.revision, original.revision + 1);
    }

    #[tokio::test]
    async fn refuses_symlink_database_path() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.sqlite");
        std::fs::write(&target, b"not a database").unwrap();
        let link = dir.path().join("linked.sqlite");
        symlink(&target, &link).unwrap();
        assert!(Workspace::open(&link).is_err());
    }
}
