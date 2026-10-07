//! Persistent provider history pages, separate from the bounded live window.
use handover_core::{ConversationId, Message, MessageId};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Page = (Vec<Message>, Option<String>);

pub(crate) struct MessageArchive(Mutex<Connection>);
impl MessageArchive {
    pub(crate) fn open_default() -> Result<Arc<Self>, Error> {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
            .ok_or("state directory unavailable")?;
        Self::open(&base.join("handover/messaging-history.sqlite"))
    }
    pub(crate) fn open(path: &Path) -> Result<Arc<Self>, Error> {
        let parent = path.parent().ok_or("archive parent unavailable")?;
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
        // Bound disk use to 1 GiB. A full archive fails writes instead of
        // claiming that history was stored. Provider page IDs remain private.
        let page_size: i64 = db.pragma_query_value(None, "page_size", |row| row.get(0))?;
        db.pragma_update(None, "max_page_count", (1024 * 1024 * 1024_i64) / page_size)?;
        db.execute_batch("
            CREATE TABLE IF NOT EXISTS messages(account TEXT, thread TEXT, id TEXT, payload TEXT NOT NULL, PRIMARY KEY(account,thread,id));
            CREATE TABLE IF NOT EXISTS pages(account TEXT, thread TEXT, before_id TEXT, ids TEXT NOT NULL, next_id TEXT, PRIMARY KEY(account,thread,before_id));")?;
        Ok(Arc::new(Self(Mutex::new(db))))
    }
    async fn run<T: Send + 'static>(
        self: &Arc<Self>,
        work: impl FnOnce(&mut Connection) -> Result<T, Error> + Send + 'static,
    ) -> Result<T, Error> {
        let archive = self.clone();
        tokio::task::spawn_blocking(move || {
            work(&mut *archive.0.lock().map_err(|_| "archive lock poisoned")?)
        })
        .await?
    }
    pub(crate) async fn merge(self: &Arc<Self>, messages: Vec<Message>) -> Result<(), Error> {
        self.run(move |db| {
            let tx = db.transaction()?;
            for message in messages {
                upsert(&tx, &message)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub(crate) async fn store_page(
        self: &Arc<Self>,
        id: ConversationId,
        before: Option<String>,
        mut messages: Vec<Message>,
        next: Option<String>,
    ) -> Result<(), Error> {
        self.run(move |db| {
            messages.sort_by(|a,b| (a.sent_at,&a.id.local_id).cmp(&(b.sent_at,&b.id.local_id)));
            let ids: Vec<_> = messages.iter().map(|message| message.id.local_id.clone()).collect();
            let tx = db.transaction()?;
            for message in messages { upsert(&tx, &message)?; }
            tx.execute("INSERT INTO pages VALUES(?1,?2,?3,?4,?5) ON CONFLICT(account,thread,before_id) DO UPDATE SET ids=excluded.ids,next_id=excluded.next_id",
                params![id.account_id.as_str(),id.local_id,before.unwrap_or_default(),serde_json::to_string(&ids)?,next])?;
            tx.commit()?; Ok(())
        }).await
    }
    pub(crate) async fn history(
        self: &Arc<Self>,
        id: ConversationId,
        limit: usize,
        before: Option<String>,
        latest: Option<Message>,
    ) -> Result<Option<Page>, Error> {
        self.run(move |db| {
            let cursor = before.clone().unwrap_or_default();
            let mut row: Option<(String,Option<String>)> = db.query_row(
                "SELECT ids,next_id FROM pages WHERE account=?1 AND thread=?2 AND before_id=?3",
                params![id.account_id.as_str(),id.local_id,cursor],|row| Ok((row.get(0)?,row.get(1)?))).optional()?;
            let exact = row.is_some();
            if row.is_none() && before.is_some() {
                row = db.query_row("SELECT ids,next_id FROM pages WHERE account=?1 AND thread=?2 AND EXISTS(SELECT 1 FROM json_each(pages.ids) WHERE value=?3) ORDER BY rowid DESC LIMIT 1",
                    params![id.account_id.as_str(),id.local_id,cursor],|row| Ok((row.get(0)?,row.get(1)?))).optional()?;
            }
            let Some((encoded,next)) = row else { return Ok(None); };
            let ids: Vec<String> = serde_json::from_str(&encoded)?;
            if before.is_none() && let (Some(latest), Some(head)) = (&latest, ids.last()) {
                let payload: Option<String> = db.query_row("SELECT payload FROM messages WHERE account=?1 AND thread=?2 AND id=?3",
                    params![id.account_id.as_str(),id.local_id,head],|row| row.get(0)).optional()?;
                let Some(payload) = payload else { return Ok(None); };
                let saved: Message = serde_json::from_str(&payload)?;
                if (latest.sent_at, &latest.id.local_id) > (saved.sent_at, &saved.id.local_id) { return Ok(None); }
            }
            let end = if exact { ids.len() } else { ids.iter().position(|id| id == &cursor).ok_or("archive cursor missing")? };
            if !exact && end == 0 { return Ok(None); }
            let start = end.saturating_sub(limit.clamp(1,100));
            let next = if start > 0 { Some(ids[start].clone()) } else { next };
            let mut messages = Vec::new();
            for local_id in &ids[start..end] {
                let payload: Option<String> = db.query_row("SELECT payload FROM messages WHERE account=?1 AND thread=?2 AND id=?3",
                    params![id.account_id.as_str(),id.local_id,local_id],|row| row.get(0)).optional()?;
                let Some(payload) = payload else { return Ok(None); };
                let message: Message = serde_json::from_str(&payload)?;
                handover_core::validate_message(&message)?;
                if message.id.conversation_id != id || &message.id.local_id != local_id {
                    return Err("archive message identity mismatch".into());
                }
                messages.push(message);
            }
            Ok(Some((messages,next)))
        }).await
    }
    pub(crate) async fn remove_message(self: &Arc<Self>, id: MessageId) -> Result<(), Error> {
        self.run(move |db| {
            db.execute(
                "DELETE FROM messages WHERE account=?1 AND thread=?2 AND id=?3",
                params![
                    id.conversation_id.account_id.as_str(),
                    id.conversation_id.local_id,
                    id.local_id
                ],
            )?;
            Ok(())
        })
        .await
    }
    pub(crate) async fn remove(
        self: &Arc<Self>,
        account: String,
        thread: Option<String>,
    ) -> Result<(), Error> {
        self.run(move |db| {
            let tx = db.transaction()?;
            for table in ["messages", "pages"] {
                tx.execute(
                    &format!("DELETE FROM {table} WHERE account=?1 AND (?2 IS NULL OR thread=?2)"),
                    params![account, thread],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }
}
fn upsert(db: &Connection, message: &Message) -> Result<(), Error> {
    db.execute("INSERT INTO messages VALUES(?1,?2,?3,?4) ON CONFLICT(account,thread,id) DO UPDATE SET payload=excluded.payload",
        params![message.id.conversation_id.account_id.as_str(),message.id.conversation_id.local_id,message.id.local_id,serde_json::to_string(message)?])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use handover_core::{MessagingAccountId, Participant};
    fn id() -> ConversationId {
        ConversationId::new(MessagingAccountId::new("fixture"), "thread")
    }
    fn message(number: u32) -> Message {
        Message {
            id: MessageId::new(id(), format!("m{number:04}")),
            sender: Participant {
                local_id: "peer".into(),
                display_name: None,
                address: None,
                is_self: false,
            },
            transport: None,
            sent_at: Some(i64::from(number)),
            text: Some("fixture".into()),
            attachments: Vec::new(),
            reply_to: None,
            reactions: Vec::new(),
            deleted: false,
        }
    }
    #[tokio::test]
    async fn history_beyond_the_live_window_survives_restart_and_supports_smaller_pages() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite");
        let archive = MessageArchive::open(&path).unwrap();
        for top in [500, 400, 300, 200, 100] {
            let before = (top < 500).then(|| format!("m{:04}", top + 1));
            let next = (top > 100).then(|| format!("m{:04}", top - 99));
            archive
                .store_page(
                    id(),
                    before,
                    ((top - 99)..=top).map(message).collect(),
                    next,
                )
                .await
                .unwrap();
        }
        drop(archive);
        let archive = MessageArchive::open(&path).unwrap();
        let mut cursor = None;
        let mut ids = std::collections::BTreeSet::new();
        loop {
            let (messages, next) = archive
                .history(id(), 20, cursor, None)
                .await
                .unwrap()
                .unwrap();
            for message in messages {
                assert!(ids.insert(message.id.local_id));
            }
            if next.is_none() {
                break;
            }
            cursor = next;
        }
        assert_eq!(ids.len(), 500);
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    #[tokio::test]
    async fn live_updates_and_explicit_removals_do_not_serve_stale_archived_pages() {
        let directory = tempfile::tempdir().unwrap();
        let archive = MessageArchive::open(&directory.path().join("history.sqlite")).unwrap();
        archive
            .store_page(id(), None, vec![message(1)], None)
            .await
            .unwrap();
        assert!(
            archive
                .history(id(), 20, None, Some(message(2)))
                .await
                .unwrap()
                .is_none()
        );
        let mut updated = message(1);
        updated.text = Some("updated".into());
        archive.merge(vec![updated.clone()]).await.unwrap();
        assert_eq!(
            archive
                .history(id(), 20, None, None)
                .await
                .unwrap()
                .unwrap()
                .0,
            vec![updated]
        );
        archive.remove_message(message(1).id).await.unwrap();
        assert!(
            archive
                .history(id(), 20, None, None)
                .await
                .unwrap()
                .is_none()
        );
        archive
            .store_page(id(), None, vec![message(2)], None)
            .await
            .unwrap();
        archive.remove("fixture".into(), None).await.unwrap();
        assert!(
            archive
                .history(id(), 20, None, None)
                .await
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn refuses_symlink_archive() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite");
        let target = directory.path().join("target");
        std::fs::write(&target, b"preserved").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(MessageArchive::open(&path).is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"preserved");
    }
}
