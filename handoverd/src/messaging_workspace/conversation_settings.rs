use super::*;
use handover_core::ConversationSettings;

const LIMIT: usize = 512;

impl Workspace {
    pub(crate) async fn conversation_settings(
        self: &Arc<Self>,
    ) -> Result<Vec<ConversationSettings>, Error> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.with_read(|store, _| match store {
            Store::Memory(state) => Ok(state.settings.values().cloned().collect()),
            Store::Database(db) => {
                let mut query = db.prepare("SELECT payload FROM conversation_settings ORDER BY account,thread LIMIT 513")?;
                let encoded = query.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
                if encoded.len() > LIMIT { return Err("conversation settings limit exceeded".into()); }
                encoded.into_iter().map(|payload| {
                    let value: ConversationSettings = serde_json::from_str(&payload)?;
                    validate_conversation_id(&value.conversation_id)?;
                    Ok(value)
                }).collect()
            }
        })).await?
    }

    pub(crate) async fn save_conversation_settings(
        self: &Arc<Self>,
        mut settings: ConversationSettings,
    ) -> Result<ConversationSettings, WorkspaceError> {
        validate_conversation_id(&settings.conversation_id)?;
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = this.store.lock().map_err(storage)?;
            let id = &settings.conversation_id;
            match &mut *guard {
                Store::Memory(state) => {
                    let current = state.settings.get(id).map_or(0, |value| value.revision);
                    if current != settings.revision { return Err(WorkspaceError::Conflict); }
                    if !state.settings.contains_key(id) && state.settings.len() >= LIMIT { return Err(WorkspaceError::Invalid("conversation settings limit reached".into())); }
                    state.revision = state.revision.checked_add(1).ok_or_else(|| storage("revision exhausted"))?;
                    settings.revision = state.revision;
                    state.settings.insert(id.clone(), settings.clone());
                }
                Store::Database(db) => {
                    let tx = db.transaction().map_err(storage)?;
                    let payload: Option<String> = tx.query_row("SELECT payload FROM conversation_settings WHERE account=?1 AND thread=?2", params![id.account_id.as_str(), id.local_id], |row| row.get(0)).optional().map_err(storage)?;
                    let current = payload.as_ref().map(|payload| serde_json::from_str::<ConversationSettings>(payload)).transpose().map_err(storage)?;
                    if current.as_ref().map_or(0, |value| value.revision) != settings.revision { return Err(WorkspaceError::Conflict); }
                    let count: i64 = tx.query_row("SELECT count(*) FROM conversation_settings", [], |row| row.get(0)).map_err(storage)?;
                    if current.is_none() && count >= LIMIT as i64 { return Err(WorkspaceError::Invalid("conversation settings limit reached".into())); }
                    let revision: i64 = tx.query_row("SELECT value FROM workspace_meta WHERE key='revision'", [], |row| row.get(0)).map_err(storage)?;
                    let revision = revision.checked_add(1).ok_or_else(|| storage("revision exhausted"))?;
                    settings.revision = revision as u64;
                    let payload = serde_json::to_string(&settings).map_err(storage)?;
                    tx.execute("UPDATE workspace_meta SET value=?1 WHERE key='revision'", [revision]).map_err(storage)?;
                    tx.execute("INSERT INTO conversation_settings(account,thread,payload) VALUES(?1,?2,?3) ON CONFLICT(account,thread) DO UPDATE SET payload=excluded.payload", params![id.account_id.as_str(), id.local_id, payload]).map_err(storage)?;
                    tx.commit().map_err(storage)?;
                }
            }
            // Retain cleared records too, so an old revision cannot restore old preferences.
            Ok(settings)
        }).await.map_err(storage)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn local_preferences_survive_restart_and_reject_stale_writes_after_clear() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("workspace.sqlite");
        let id = ConversationId::new(handover_core::MessagingAccountId::new("fixture"), "thread");
        let workspace = Workspace::open(&path).unwrap();
        let mut settings = ConversationSettings::new(id.clone());
        settings.pinned = true;
        settings.muted = true;
        settings.unread_reminder = true;
        let saved = workspace
            .save_conversation_settings(settings.clone())
            .await
            .unwrap();
        assert!(
            workspace
                .save_conversation_settings(settings)
                .await
                .is_err()
        );
        drop(workspace);
        let workspace = Workspace::open(&path).unwrap();
        assert_eq!(
            workspace.conversation_settings().await.unwrap(),
            vec![saved.clone()]
        );
        let mut cleared = ConversationSettings::new(id);
        cleared.revision = saved.revision;
        let cleared = workspace.save_conversation_settings(cleared).await.unwrap();
        assert!(cleared.revision > saved.revision);
        assert!(workspace.save_conversation_settings(saved).await.is_err());
        workspace.remove_account("fixture".into()).await.unwrap();
        assert!(workspace.conversation_settings().await.unwrap().is_empty());
    }
}
