use super::*;
use handover_core::{Attachment, MessageAttachmentDraft};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
};

const FILE_LIMIT: u64 = 50 * 1024 * 1024;
const TOTAL_LIMIT: u64 = 256 * 1024 * 1024;
const COUNT_LIMIT: usize = 32;
const RECORD_LIMIT: usize = 4096;

pub(super) struct FileRoot {
    path: PathBuf,
    _temporary: Option<tempfile::TempDir>,
    _lock: Option<std::fs::File>,
}
impl FileRoot {
    pub(super) fn temporary() -> Option<Self> {
        let directory = tempfile::Builder::new()
            .prefix("handover-drafts-")
            .tempdir()
            .ok()?;
        Some(Self {
            path: directory.path().to_owned(),
            _temporary: Some(directory),
            _lock: None,
        })
    }
    pub(super) fn persistent(parent: &Path) -> Result<Self, Error> {
        let path = std::fs::canonicalize(parent)?.join("messaging-draft-files");
        if !path.exists() {
            std::fs::create_dir(&path)?;
        }
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("draft file directory is invalid".into());
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path.join(".lock"))?;
        use std::os::fd::AsRawFd;
        // SAFETY: flock only borrows the valid descriptor for this call.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("attachment draft storage is already in use".into());
        }
        Ok(Self {
            path,
            _temporary: None,
            _lock: Some(lock),
        })
    }
}

impl Workspace {
    pub(crate) async fn attachment_draft(
        self: &Arc<Self>,
        id: ConversationId,
    ) -> Result<MessageAttachmentDraft, Error> {
        validate_conversation_id(&id)?;
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            this.with_read(|store, _| {
                let draft = load(store, &id)?;
                if let Some(root) = this.files.as_ref() {
                    validate_saved(&root.path, &draft)?;
                }
                Ok(draft)
            })
        })
        .await?
    }
    #[cfg(test)]
    pub(crate) async fn save_attachment_draft(
        self: &Arc<Self>,
        id: ConversationId,
        file_url: Option<String>,
        mime: Option<String>,
        revision: u64,
    ) -> Result<MessageAttachmentDraft, WorkspaceError> {
        self.save_attachment_queue(id, file_url, mime, Vec::new(), revision)
            .await
    }
    pub(crate) async fn save_attachment_queue(
        self: &Arc<Self>,
        id: ConversationId,
        file_url: Option<String>,
        mime: Option<String>,
        queued: Vec<handover_core::messaging::AttachmentDraftSource>,
        revision: u64,
    ) -> Result<MessageAttachmentDraft, WorkspaceError> {
        validate_conversation_id(&id)?;
        if queued.len() >= 10 || (file_url.is_none() && !queued.is_empty()) {
            return Err(WorkspaceError::Invalid(
                "attachment queue exceeds limits".into(),
            ));
        }
        if queued.iter().any(|file| {
            file.mime
                .as_ref()
                .is_some_and(|mime| mime.len() > 256 || mime.chars().any(char::is_control))
        }) {
            return Err(WorkspaceError::Invalid("invalid MIME type".into()));
        }
        if mime
            .as_ref()
            .is_some_and(|mime| mime.len() > 256 || mime.chars().any(char::is_control))
        {
            return Err(WorkspaceError::Invalid("invalid MIME type".into()));
        }
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            this.save_attachment_blocking(id, file_url, mime, queued, revision)
        })
        .await
        .map_err(storage)?
    }
    fn save_attachment_blocking(
        &self,
        id: ConversationId,
        file_url: Option<String>,
        mime: Option<String>,
        queued: Vec<handover_core::messaging::AttachmentDraftSource>,
        revision: u64,
    ) -> Result<MessageAttachmentDraft, WorkspaceError> {
        let root = self
            .files
            .as_ref()
            .ok_or_else(|| storage("draft file storage unavailable"))?;
        let mut guard = self.store.lock().map_err(storage)?;
        let current = load(&mut guard, &id).map_err(storage)?;
        if current.revision != revision {
            return Err(WorkspaceError::Conflict);
        }
        if current.revision == 0 {
            let records = match &*guard {
                Store::Memory(state) => state.attachments.len() as i64,
                Store::Database(db) => db
                    .query_row("SELECT count(*) FROM attachment_drafts", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .map_err(storage)?,
            };
            if records >= RECORD_LIMIT as i64 {
                return Err(WorkspaceError::Invalid(
                    "attachment draft record limit reached".into(),
                ));
            }
        }
        let drafts = all(&mut guard).map_err(storage)?;
        if file_url.is_some() && current.attachment.is_none() && drafts.len() >= COUNT_LIMIT {
            return Err(WorkspaceError::Invalid(
                "attachment draft limit reached".into(),
            ));
        }
        clean(&root.path, &drafts).map_err(storage)?;
        let mut directories = Vec::new();
        let mut attachments = Vec::new();
        let files = file_url
            .into_iter()
            .map(|file_url| handover_core::messaging::AttachmentDraftSource {
                file_url,
                mime: mime.clone(),
            })
            .chain(queued);
        let mut added_size = 0u64;
        for source_file in files {
            let file_url = source_file.file_url;
            let mime = source_file.mime;
            let url = url::Url::parse(&file_url)
                .map_err(|_| WorkspaceError::Invalid("invalid file URL".into()))?;
            if url.scheme() != "file" || url.query().is_some() || url.fragment().is_some() {
                return Err(WorkspaceError::Invalid("invalid file URL".into()));
            }
            let source = url
                .to_file_path()
                .map_err(|_| WorkspaceError::Invalid("invalid file URL".into()))?;
            let name = source
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(handover_core::sanitize_file_name)
                .ok_or_else(|| WorkspaceError::Invalid("invalid file name".into()))?;
            let mut input = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(&source)
                .map_err(storage)?;
            let metadata = input.metadata().map_err(storage)?;
            if !metadata.is_file() || metadata.len() > FILE_LIMIT {
                return Err(WorkspaceError::Invalid(
                    "attachment file exceeds limits".into(),
                ));
            }
            let old_size = current
                .attachment
                .iter()
                .chain(&current.queued_attachments)
                .map(|part| part.size_bytes.unwrap_or(FILE_LIMIT))
                .sum::<u64>();
            let used = drafts
                .iter()
                .flat_map(|draft| draft.attachment.iter().chain(&draft.queued_attachments))
                .map(|part| part.size_bytes.unwrap_or(FILE_LIMIT))
                .sum::<u64>();
            added_size = added_size.saturating_add(metadata.len());
            if used.saturating_sub(old_size).saturating_add(added_size) > TOTAL_LIMIT {
                return Err(WorkspaceError::Invalid(
                    "attachment draft storage is full".into(),
                ));
            }
            let staged = tempfile::Builder::new()
                .prefix(".draft-")
                .tempdir_in(&root.path)
                .map_err(storage)?;
            let path = staged.path().join(&name);
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .map_err(storage)?;
            let copied = std::io::copy(&mut (&mut input).take(FILE_LIMIT + 1), &mut output)
                .map_err(storage)?;
            if copied > FILE_LIMIT || copied != metadata.len() {
                return Err(WorkspaceError::Invalid(
                    "attachment changed while saving".into(),
                ));
            }
            output.flush().map_err(storage)?;
            output.sync_all().map_err(storage)?;
            std::fs::File::open(staged.path())
                .and_then(|dir| dir.sync_all())
                .map_err(storage)?;
            std::fs::File::open(&root.path)
                .and_then(|dir| dir.sync_all())
                .map_err(storage)?;
            let handle = staged
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| storage("draft handle unavailable"))?
                .to_owned();
            let path = path
                .to_str()
                .ok_or_else(|| storage("draft path unavailable"))?
                .to_owned();
            directories.push(staged);
            attachments.push(Attachment {
                local_id: handle,
                mime,
                name: Some(name),
                size_bytes: Some(copied),
                staged_path: Some(path),
            });
        }
        let attachment = if attachments.is_empty() {
            None
        } else {
            Some(attachments.remove(0))
        };
        let mut saved = MessageAttachmentDraft {
            conversation_id: id,
            attachment,
            queued_attachments: attachments,
            revision: 0,
        };
        match &mut *guard {
            Store::Memory(state) => {
                state.revision = state
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| storage("revision exhausted"))?;
                saved.revision = state.revision;
                state
                    .attachments
                    .insert(saved.conversation_id.clone(), saved.clone());
            }
            Store::Database(db) => {
                let tx = db.transaction().map_err(storage)?;
                let value: i64 = tx
                    .query_row(
                        "SELECT value FROM workspace_meta WHERE key='revision'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(storage)?;
                let value = value
                    .checked_add(1)
                    .ok_or_else(|| storage("revision exhausted"))?;
                saved.revision = value as u64;
                tx.execute(
                    "UPDATE workspace_meta SET value=?1 WHERE key='revision'",
                    [value],
                )
                .map_err(storage)?;
                let payload = serde_json::to_string(&saved).map_err(storage)?;
                tx.execute("INSERT INTO attachment_drafts VALUES(?1,?2,?3,?4) ON CONFLICT(account,thread) DO UPDATE SET payload=excluded.payload,revision=excluded.revision",params![saved.conversation_id.account_id.as_str(),saved.conversation_id.local_id,payload,value]).map_err(storage)?;
                tx.commit().map_err(storage)?;
            }
        }
        for directory in directories {
            let _ = directory.keep();
        }
        clean(&root.path, &all(&mut guard).map_err(storage)?).map_err(storage)?;
        Ok(saved)
    }
    pub(super) fn clean_attachment_files(&self) -> Result<(), Error> {
        let Some(root) = self.files.as_ref() else {
            return Ok(());
        };
        self.with_read(|store, _| clean(&root.path, &all(store)?))
    }
}
fn empty(id: ConversationId) -> MessageAttachmentDraft {
    MessageAttachmentDraft {
        conversation_id: id,
        attachment: None,
        queued_attachments: Vec::new(),
        revision: 0,
    }
}
fn load(store: &mut Store, id: &ConversationId) -> Result<MessageAttachmentDraft, Error> {
    match store {
        Store::Memory(state) => Ok(state
            .attachments
            .get(id)
            .cloned()
            .unwrap_or_else(|| empty(id.clone()))),
        Store::Database(db) => {
            let payload: Option<String> = db
                .query_row(
                    "SELECT payload FROM attachment_drafts WHERE account=?1 AND thread=?2",
                    params![id.account_id.as_str(), id.local_id],
                    |row| row.get(0),
                )
                .optional()?;
            let draft = payload
                .map(|payload| serde_json::from_str::<MessageAttachmentDraft>(&payload))
                .transpose()?
                .unwrap_or_else(|| empty(id.clone()));
            if &draft.conversation_id != id {
                return Err("attachment draft identity mismatch".into());
            }
            Ok(draft)
        }
    }
}
fn all(store: &mut Store) -> Result<Vec<MessageAttachmentDraft>, Error> {
    match store {
        Store::Memory(state) => Ok(state
            .attachments
            .values()
            .filter(|draft| draft.attachment.is_some())
            .cloned()
            .collect()),
        Store::Database(db) => {
            let mut statement = db.prepare("SELECT payload FROM attachment_drafts WHERE json_extract(payload,'$.attachment') IS NOT NULL")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
        }
    }
}
fn clean(root: &Path, drafts: &[MessageAttachmentDraft]) -> Result<(), Error> {
    if drafts.len() > COUNT_LIMIT {
        return Err("attachment draft count exceeds limit".into());
    }
    for draft in drafts {
        validate_saved(root, draft)?;
    }
    let kept = drafts
        .iter()
        .flat_map(|draft| draft.attachment.iter().chain(&draft.queued_attachments))
        .filter_map(|part| part.staged_path.as_ref())
        .map(PathBuf::from)
        .filter_map(|path| path.parent().map(Path::to_owned))
        .collect::<BTreeSet<_>>();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(".draft-"))
            && !kept.contains(&entry.path())
        {
            std::fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}
fn validate_saved(root: &Path, draft: &MessageAttachmentDraft) -> Result<(), Error> {
    validate_conversation_id(&draft.conversation_id)?;
    if draft.queued_attachments.len() >= 10
        || (draft.attachment.is_none() && !draft.queued_attachments.is_empty())
    {
        return Err("invalid saved attachment queue".into());
    }
    for part in draft.attachment.iter().chain(&draft.queued_attachments) {
        let path = part
            .staged_path
            .as_ref()
            .map(Path::new)
            .ok_or("attachment draft path missing")?;
        let directory = path.parent().ok_or("attachment draft directory missing")?;
        if draft.revision == 0
            || directory.parent() != Some(root)
            || directory.file_name().and_then(|value| value.to_str())
                != Some(part.local_id.as_str())
            || !part.local_id.starts_with(".draft-")
            || part.size_bytes.is_none_or(|size| size > FILE_LIMIT)
            || part.name.as_ref().is_none_or(|name| {
                handover_core::sanitize_file_name(name).as_ref() != Some(name)
                    || path.file_name().and_then(|value| value.to_str()) != Some(name.as_str())
            })
            || part
                .mime
                .as_ref()
                .is_some_and(|mime| mime.len() > 256 || mime.chars().any(char::is_control))
        {
            return Err("invalid saved attachment draft".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id() -> ConversationId {
        ConversationId::new(handover_core::MessagingAccountId::new("fixture"), "thread")
    }
    #[tokio::test]
    async fn queued_files_survive_restart_and_failed_updates_preserve_the_batch() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("workspace.sqlite");
        let a = directory.path().join("program.cc");
        let b = directory.path().join("photo.png");
        std::fs::write(&a, b"int main() { return 0; }\n").unwrap();
        std::fs::write(&b, b"original image bytes").unwrap();
        let uri = |path: &Path| url::Url::from_file_path(path).unwrap().to_string();
        let workspace = Workspace::open(&database).unwrap();
        let saved = workspace
            .save_attachment_queue(
                id(),
                Some(uri(&a)),
                Some("text/x-c++src".into()),
                vec![handover_core::messaging::AttachmentDraftSource {
                    file_url: uri(&b),
                    mime: Some("image/png".into()),
                }],
                0,
            )
            .await
            .unwrap();
        assert_eq!(
            saved.attachment.as_ref().unwrap().name.as_deref(),
            Some("program.cc")
        );
        assert_eq!(saved.queued_attachments.len(), 1);
        let paths = saved
            .attachment
            .iter()
            .chain(&saved.queued_attachments)
            .map(|part| PathBuf::from(part.staged_path.as_ref().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(
            std::fs::read(&paths[0]).unwrap(),
            std::fs::read(&a).unwrap()
        );
        assert_eq!(
            std::fs::read(&paths[1]).unwrap(),
            std::fs::read(&b).unwrap()
        );
        let failed = workspace
            .save_attachment_queue(
                id(),
                Some(uri(&a)),
                None,
                vec![handover_core::messaging::AttachmentDraftSource {
                    file_url: uri(&directory.path().join("missing")),
                    mime: None,
                }],
                saved.revision,
            )
            .await;
        assert!(failed.is_err());
        assert_eq!(workspace.attachment_draft(id()).await.unwrap(), saved);
        drop(workspace);
        let workspace = Workspace::open(&database).unwrap();
        assert_eq!(workspace.attachment_draft(id()).await.unwrap(), saved);
        workspace
            .save_attachment_draft(id(), None, None, saved.revision)
            .await
            .unwrap();
        assert!(paths.iter().all(|path| !path.exists()));
    }

    #[tokio::test]
    async fn attachment_limits_preserve_saved_files_and_removal_reopens_a_slot() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(&directory.path().join("workspace.sqlite")).unwrap();
        let source = directory.path().join("file.txt");
        std::fs::write(&source, b"fixture").unwrap();
        let uri = url::Url::from_file_path(&source).unwrap().to_string();
        let mut saved = Vec::new();
        for number in 0..COUNT_LIMIT {
            let id = ConversationId::new(
                handover_core::MessagingAccountId::new("fixture"),
                format!("thread-{number}"),
            );
            saved.push(
                workspace
                    .save_attachment_draft(id, Some(uri.clone()), None, 0)
                    .await
                    .unwrap(),
            );
        }
        assert!(matches!(
            workspace
                .save_attachment_draft(id(), Some(uri.clone()), None, 0)
                .await,
            Err(WorkspaceError::Invalid(_))
        ));
        let oversized = directory.path().join("oversized.bin");
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(FILE_LIMIT + 1)
            .unwrap();
        assert!(
            workspace
                .save_attachment_draft(
                    saved[0].conversation_id.clone(),
                    Some(url::Url::from_file_path(&oversized).unwrap().to_string()),
                    None,
                    saved[0].revision
                )
                .await
                .is_err()
        );
        assert_eq!(
            workspace
                .attachment_draft(saved[0].conversation_id.clone())
                .await
                .unwrap(),
            saved[0]
        );
        workspace
            .save_attachment_draft(
                saved[0].conversation_id.clone(),
                None,
                None,
                saved[0].revision,
            )
            .await
            .unwrap();
        workspace
            .save_attachment_draft(id(), Some(uri), None, 0)
            .await
            .unwrap();
        assert_eq!(
            workspace.get().await.unwrap().draft_conversations.len(),
            COUNT_LIMIT
        );
    }

    #[tokio::test]
    async fn attachment_copy_survives_source_removal_and_workspace_restart_and_uses_cas() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("workspace.sqlite");
        let file = dir.path().join("photo.png");
        std::fs::write(&file, b"fixture").unwrap();
        let workspace = Workspace::open(&db).unwrap();
        let saved = workspace
            .save_attachment_draft(
                id(),
                Some(url::Url::from_file_path(&file).unwrap().to_string()),
                Some("image/png".into()),
                0,
            )
            .await
            .unwrap();
        let path = PathBuf::from(
            saved
                .attachment
                .as_ref()
                .unwrap()
                .staged_path
                .as_ref()
                .unwrap(),
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            workspace
                .get()
                .await
                .unwrap()
                .draft_conversations
                .contains(&id())
        );
        std::fs::remove_file(file).unwrap();
        drop(workspace);
        let workspace = Workspace::open(&db).unwrap();
        let recovered = workspace.attachment_draft(id()).await.unwrap();
        assert_eq!(recovered, saved);
        assert_eq!(std::fs::read(&path).unwrap(), b"fixture");
        assert!(matches!(
            workspace.save_attachment_draft(id(), None, None, 0).await,
            Err(WorkspaceError::Conflict)
        ));
        workspace
            .save_attachment_draft(id(), None, None, saved.revision)
            .await
            .unwrap();
        assert!(!path.exists());
        assert!(
            !workspace
                .get()
                .await
                .unwrap()
                .draft_conversations
                .contains(&id())
        );
        assert!(
            matches!(
                workspace.save_attachment_draft(id(), None, None, 0).await,
                Err(WorkspaceError::Conflict)
            ),
            "clearing retains a revision to reject stale initial writes"
        );
    }
    #[tokio::test]
    async fn invalid_sources_do_not_replace_a_saved_attachment_and_logout_cleans_files() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(&dir.path().join("workspace.sqlite")).unwrap();
        let file = dir.path().join("file.txt");
        std::fs::write(&file, b"fixture").unwrap();
        let saved = workspace
            .save_attachment_draft(
                id(),
                Some(url::Url::from_file_path(&file).unwrap().to_string()),
                None,
                0,
            )
            .await
            .unwrap();
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        for url in [
            "https://example.test/file".into(),
            url::Url::from_file_path(link).unwrap().to_string(),
        ] {
            assert!(
                workspace
                    .save_attachment_draft(id(), Some(url), None, saved.revision)
                    .await
                    .is_err()
            );
            assert_eq!(workspace.attachment_draft(id()).await.unwrap(), saved);
        }
        let path = saved.attachment.unwrap().staged_path.unwrap();
        workspace
            .remove_account(id().account_id.as_str().into())
            .await
            .unwrap();
        assert!(!Path::new(&path).exists());
    }
}
