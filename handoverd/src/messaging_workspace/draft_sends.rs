use super::*;

impl Workspace {
    /// Bind before submission, or resolve using evidence checked by the IPC handler.
    pub(crate) async fn draft_send(
        self: &Arc<Self>,
        draft: MessageDraft,
        operation: String,
        sent: Option<bool>,
    ) -> Result<MessageDraft, WorkspaceError> {
        validate_draft(&draft)?;
        if operation.is_empty() || operation.len() > 256 {
            return Err(WorkspaceError::Invalid("invalid operation".into()));
        }
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = this.store.lock().map_err(storage)?;
            match &mut *store {
                Store::Memory(state) => {
                    let current = state.drafts.get(&draft.conversation_id).cloned()
                        .unwrap_or_else(|| empty_draft(draft.conversation_id.clone()));
                    check(&current, &draft, &operation, sent)?;
                    if current.revision == 0 && state.drafts.len() >= MAX_DRAFTS {
                        return Err(WorkspaceError::Invalid("draft limit reached".into()));
                    }
                    state.revision = state.revision.checked_add(1).ok_or_else(|| storage("revision exhausted"))?;
                    let saved = transition(draft, operation, sent, state.revision);
                    if sent == Some(true) {
                        state.drafts.remove(&saved.conversation_id);
                    } else {
                        state.drafts.insert(saved.conversation_id.clone(), saved.clone());
                    }
                    Ok(saved)
                }
                Store::Database(db) => {
                    let tx = db.transaction().map_err(storage)?;
                    let current = load_draft_tx(&tx, &draft.conversation_id).map_err(storage)?;
                    check(&current, &draft, &operation, sent)?;
                    let count: i64 = tx.query_row("SELECT count(*) FROM drafts", [], |row| row.get(0)).map_err(storage)?;
                    if current.revision == 0 && count >= MAX_DRAFTS as i64 {
                        return Err(WorkspaceError::Invalid("draft limit reached".into()));
                    }
                    let revision: i64 = tx.query_row("SELECT value FROM workspace_meta WHERE key='revision'", [], |row| row.get(0)).map_err(storage)?;
                    let revision = revision.checked_add(1).ok_or_else(|| storage("revision exhausted"))?;
                    let saved = transition(draft, operation, sent, revision as u64);
                    tx.execute("UPDATE workspace_meta SET value=?1 WHERE key='revision'", [revision]).map_err(storage)?;
                    if sent == Some(true) {
                        tx.execute("DELETE FROM drafts WHERE account=?1 AND thread=?2", params![saved.conversation_id.account_id.as_str(), saved.conversation_id.local_id]).map_err(storage)?;
                    } else {
                        tx.execute("INSERT INTO drafts(account,thread,text,reply_to,revision,pending_operation) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(account,thread) DO UPDATE SET text=excluded.text,reply_to=excluded.reply_to,revision=excluded.revision,pending_operation=excluded.pending_operation", params![saved.conversation_id.account_id.as_str(), saved.conversation_id.local_id, saved.text, saved.reply_to.as_ref().map(|id| id.local_id.as_str()), revision, saved.pending_operation]).map_err(storage)?;
                    }
                    tx.commit().map_err(storage)?;
                    Ok(saved)
                }
            }
        }).await.map_err(storage)?
    }
}

fn check(
    current: &MessageDraft,
    draft: &MessageDraft,
    operation: &str,
    sent: Option<bool>,
) -> Result<(), WorkspaceError> {
    let valid = if sent.is_some() {
        current == draft && current.pending_operation.as_deref() == Some(operation)
    } else {
        current.revision == draft.revision
            && current.pending_operation.is_none()
            && draft.pending_operation.is_none()
            && !draft.text.trim().is_empty()
    };
    if valid {
        Ok(())
    } else {
        Err(WorkspaceError::Conflict)
    }
}

fn transition(
    mut draft: MessageDraft,
    operation: String,
    sent: Option<bool>,
    revision: u64,
) -> MessageDraft {
    if sent == Some(true) {
        return empty_draft(draft.conversation_id);
    }
    draft.revision = revision;
    draft.pending_operation = sent.is_none().then_some(operation);
    draft
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn bound_text_survives_restart_blocks_replay_and_keeps_reply_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.sqlite");
        let id = ConversationId::new(handover_core::MessagingAccountId::new("test"), "thread");
        let draft = MessageDraft {
            conversation_id: id.clone(),
            text: "keep this".into(),
            reply_to: Some(MessageId::new(id.clone(), "original")),
            revision: 0,
            pending_operation: None,
        };
        let ws = Workspace::open(&path).unwrap();
        let bound = ws
            .draft_send(draft.clone(), "send-first".into(), None)
            .await
            .unwrap();
        assert!(
            ws.draft_send(draft.clone(), "send-again".into(), None)
                .await
                .is_err()
        );
        assert!(
            ws.save(MessageDraft {
                pending_operation: None,
                ..bound.clone()
            })
            .await
            .is_err()
        );
        drop(ws);
        let ws = Workspace::open(&path).unwrap();
        assert_eq!(ws.draft(id.clone()).await.unwrap(), bound);
        let retry = ws
            .draft_send(bound, "send-first".into(), Some(false))
            .await
            .unwrap();
        assert_eq!(retry.text, draft.text);
        assert_eq!(retry.reply_to, draft.reply_to);
        assert!(retry.pending_operation.is_none());
        let bound = ws
            .draft_send(retry.clone(), "send-second".into(), None)
            .await
            .unwrap();
        assert!(
            ws.draft_send(retry, "send-second".into(), Some(true))
                .await
                .is_err()
        );
        let cleared = ws
            .draft_send(bound, "send-second".into(), Some(true))
            .await
            .unwrap();
        assert!(cleared.text.is_empty());
        assert_eq!(ws.draft(id).await.unwrap(), cleared);
    }
}
