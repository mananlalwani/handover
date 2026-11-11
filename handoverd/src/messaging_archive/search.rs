use super::*;
use handover_core::MessageSearchHit;

fn index(db: &mut Connection) -> Result<(), Error> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='message_search')",
        [],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(());
    }
    let tx = db.transaction()?;
    tx.execute_batch("CREATE VIRTUAL TABLE message_search USING fts5(text);
        CREATE TRIGGER search_insert AFTER INSERT ON messages BEGIN
            INSERT INTO message_search(rowid,text) SELECT new.rowid,json_extract(new.payload,'$.text')
            WHERE coalesce(json_extract(new.payload,'$.deleted'),0)=0;
        END;
        CREATE TRIGGER search_update AFTER UPDATE ON messages BEGIN
            DELETE FROM message_search WHERE rowid=old.rowid;
            INSERT INTO message_search(rowid,text) SELECT new.rowid,json_extract(new.payload,'$.text')
            WHERE coalesce(json_extract(new.payload,'$.deleted'),0)=0;
        END;
        CREATE TRIGGER search_delete AFTER DELETE ON messages BEGIN
            DELETE FROM message_search WHERE rowid=old.rowid;
        END;
        INSERT INTO message_search(rowid,text) SELECT rowid,json_extract(payload,'$.text') FROM messages
            WHERE coalesce(json_extract(payload,'$.deleted'),0)=0;")?;
    tx.commit()?;
    Ok(())
}

impl MessageArchive {
    pub(crate) async fn search(
        self: &Arc<Self>,
        query: String,
        account: Option<String>,
        offset: u32,
    ) -> Result<(Vec<MessageSearchHit>, bool), Error> {
        if query.chars().count() > 256 || query.split_whitespace().count() > 16 || offset > 10_000 {
            return Err("invalid search bounds".into());
        }
        let expression = query
            .split_whitespace()
            .filter(|word| word.chars().any(char::is_alphanumeric))
            .map(|word| format!("\"{}\"*", word.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ");
        if expression.is_empty() {
            return Ok((Vec::new(), false));
        }
        self.run(move |db| {
            index(db)?;
            let mut statement = db.prepare(
                "SELECT m.payload FROM message_search s
                JOIN messages m ON m.rowid=s.rowid WHERE message_search MATCH ?1
                AND (?2 IS NULL OR m.account=?2)
                ORDER BY bm25(message_search),coalesce(json_extract(m.payload,'$.sent_at'),0) DESC,
                    m.account,m.thread,m.id LIMIT 26 OFFSET ?3",
            )?;
            let rows = statement.query_map(params![expression, account, offset], |row| {
                row.get::<_, String>(0)
            })?;
            let mut hits = Vec::new();
            for row in rows {
                let message: Message = serde_json::from_str(&row?)?;
                handover_core::validate_message(&message)?;
                if message.deleted {
                    continue;
                }
                hits.push(MessageSearchHit {
                    message_id: message.id,
                    snippet: message.text.unwrap_or_default().chars().take(280).collect(),
                    sender: message.sender,
                    sent_at: message.sent_at,
                });
            }
            let more = hits.len() > 25;
            hits.truncate(25);
            Ok((hits, more))
        })
        .await
    }

    pub(crate) async fn previews(
        self: &Arc<Self>,
        ids: Vec<ConversationId>,
    ) -> Result<Vec<Message>, Error> {
        if ids.len() > 50 {
            return Err("too many preview conversations".into());
        }
        self.run(move |db| {
            let mut statement = db.prepare(
                "SELECT payload FROM messages WHERE account=?1 AND thread=?2
                ORDER BY coalesce(json_extract(payload,'$.sent_at'),0) DESC,id DESC LIMIT 1",
            )?;
            let mut messages = Vec::new();
            for id in ids {
                let payload: Option<String> = statement
                    .query_row(params![id.account_id.as_str(), id.local_id], |row| {
                        row.get(0)
                    })
                    .optional()?;
                if let Some(payload) = payload {
                    let message: Message = serde_json::from_str(&payload)?;
                    handover_core::validate_message(&message)?;
                    if message.id.conversation_id != id {
                        return Err("archive identity mismatch".into());
                    }
                    messages.push(message);
                }
            }
            Ok(messages)
        })
        .await
    }

    #[cfg(test)]
    pub(crate) async fn context(
        self: &Arc<Self>,
        id: MessageId,
    ) -> Result<Option<Vec<Message>>, Error> {
        Ok(self.context_page(id).await?.map(|(messages, _)| messages))
    }
    pub(crate) async fn context_page(
        self: &Arc<Self>,
        id: MessageId,
    ) -> Result<Option<Page>, Error> {
        self.run(move |db| {
            let anchor: Option<String> = db.query_row("SELECT payload FROM messages
                WHERE account=?1 AND thread=?2 AND id=?3", params![id.conversation_id.account_id.as_str(),
                id.conversation_id.local_id,id.local_id], |row| row.get(0)).optional()?;
            let Some(anchor) = anchor else { return Ok(None); };
            let anchor: Message = serde_json::from_str(&anchor)?;
            handover_core::validate_message(&anchor)?;
            if anchor.id != id { return Err("archive identity mismatch".into()); }
            let mut messages = Vec::new();
            for (comparison, order) in [("<=", "DESC"), (">", "ASC")] {
                let sql = format!("SELECT payload FROM messages WHERE account=?1 AND thread=?2
                    AND (coalesce(json_extract(payload,'$.sent_at'),0),id) {comparison} (?3,?4)
                    ORDER BY coalesce(json_extract(payload,'$.sent_at'),0) {order},id {order} LIMIT 11");
                let mut statement = db.prepare(&sql)?;
                for row in statement.query_map(params![id.conversation_id.account_id.as_str(),
                    id.conversation_id.local_id,anchor.sent_at.unwrap_or(0), id.local_id], |row| row.get::<_,String>(0))? {
                    let message: Message = serde_json::from_str(&row?)?;
                    handover_core::validate_message(&message)?;
                    if message.id.conversation_id != id.conversation_id { return Err("archive identity mismatch".into()); }
                    messages.push(message);
                }
            }
            messages.sort_by(|a,b| (a.sent_at,&a.id.local_id).cmp(&(b.sent_at,&b.id.local_id)));
            let cursor = if let Some(first) = messages.first() {
                let page: Option<(String, Option<String>)> = db.query_row(
                    "SELECT ids,next_id FROM pages WHERE account=?1 AND thread=?2
                     AND EXISTS(SELECT 1 FROM json_each(pages.ids) WHERE value=?3)
                     ORDER BY rowid DESC LIMIT 1",
                    params![id.conversation_id.account_id.as_str(), id.conversation_id.local_id, first.id.local_id],
                    |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
                if let Some((ids, next)) = page {
                    let ids: Vec<String> = serde_json::from_str(&ids)?;
                    match ids.iter().position(|value| value == &first.id.local_id) {
                        Some(0) => next,
                        Some(_) => Some(first.id.local_id.clone()),
                        None => return Err("archive page identity mismatch".into()),
                    }
                } else { None }
            } else { None };
            Ok(Some((messages, cursor)))
        }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use handover_core::{MessagingAccountId, Participant};
    fn message(n: u32, text: &str) -> Message {
        Message {
            id: MessageId::new(
                ConversationId::new(MessagingAccountId::new("fixture"), "thread"),
                format!("{n:04}"),
            ),
            text: Some(text.into()),
            sender: Participant {
                local_id: "peer".into(),
                display_name: None,
                address: None,
                is_self: false,
            },
            sent_at: Some(i64::from(n)),
            transport: None,
            attachments: vec![],
            reply_to: None,
            reactions: vec![],
            deleted: false,
        }
    }
    #[tokio::test]
    async fn context_continues_through_stored_pages_without_inventing_provider_cursors() {
        let directory = tempfile::tempdir().unwrap();
        let archive = MessageArchive::open(&directory.path().join("archive.sqlite")).unwrap();
        let id = message(45, "anchor").id;
        archive
            .store_page(
                id.conversation_id.clone(),
                None,
                (20..70).map(|n| message(n, "saved history")).collect(),
                Some("provider-older".into()),
            )
            .await
            .unwrap();
        let (context, cursor) = archive.context_page(id.clone()).await.unwrap().unwrap();
        let oldest = context.first().unwrap().id.clone();
        assert_eq!(cursor.as_deref(), Some(oldest.local_id.as_str()));
        let (previous, next) = archive
            .history(id.conversation_id.clone(), 50, cursor, None)
            .await
            .unwrap()
            .unwrap();
        assert!(
            previous
                .iter()
                .all(|message| message.sent_at < context.first().unwrap().sent_at)
        );
        assert_eq!(next.as_deref(), Some("provider-older"));
        let (_, cursor) = archive
            .context_page(message(20, "").id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cursor.as_deref(), Some("provider-older"));
    }
    #[tokio::test]
    async fn saved_search_indexes_existing_and_live_messages_and_reconciles_deletion() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive.sqlite");
        let archive = MessageArchive::open(&path).unwrap();
        archive
            .merge((0..60).map(|n| message(n, "Café train ticket")).collect())
            .await
            .unwrap();
        let (hits, more) = archive.search("cafe train".into(), None, 0).await.unwrap();
        assert_eq!(hits.len(), 25);
        assert!(more);
        let anchor = hits[0].message_id.clone();
        assert!(
            archive
                .context(anchor.clone())
                .await
                .unwrap()
                .unwrap()
                .iter()
                .any(|m| m.id == anchor)
        );
        assert!(
            archive
                .search("ticket".into(), Some("other".into()), 0)
                .await
                .unwrap()
                .0
                .is_empty()
        );
        let mut changed = message(59, "Replaced content");
        archive.merge(vec![changed.clone()]).await.unwrap();
        assert_eq!(
            archive
                .search("replac".into(), None, 0)
                .await
                .unwrap()
                .0
                .len(),
            1
        );
        changed.deleted = true;
        archive.merge(vec![changed]).await.unwrap();
        assert!(
            archive
                .search("replac".into(), None, 0)
                .await
                .unwrap()
                .0
                .is_empty()
        );
        archive.remove_message(message(58, "").id).await.unwrap();
        drop(archive);
        let archive = MessageArchive::open(&path).unwrap();
        assert_eq!(
            archive
                .search("ticket".into(), None, 50)
                .await
                .unwrap()
                .0
                .len(),
            8
        );
        archive.remove("fixture".into(), None).await.unwrap();
        assert!(
            archive
                .search("ticket".into(), None, 0)
                .await
                .unwrap()
                .0
                .is_empty()
        );
        assert!(archive.search("\" OR *".into(), None, 0).await.is_ok());
    }
}
