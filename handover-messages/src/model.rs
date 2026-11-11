use handover_core::*;
use handover_ipc::ServerPayload;
use std::collections::BTreeMap;

pub struct Draft {
    pub value: MessageDraft,
    pub loaded: bool,
    pub dirty: bool,
    pub saving: bool,
    pub conflict: bool,
    pub error: String,
}

impl Draft {
    fn new(id: ConversationId) -> Self {
        Self {
            value: MessageDraft {
                conversation_id: id,
                text: String::new(),
                reply_to: None,
                revision: 0,
            },
            loaded: false,
            dirty: false,
            saving: false,
            conflict: false,
            error: String::new(),
        }
    }
}

#[derive(Default)]
pub struct Model {
    pub connected: bool,
    pub accounts: Vec<MessagingAccount>,
    pub conversations: Vec<Conversation>,
    pub selected: Option<ConversationId>,
    pub anchor: Option<MessageId>,
    pub scroll_anchor: Option<MessageId>,
    pub browsing_older: bool,
    pub messages: Vec<Message>,
    pub previews: BTreeMap<ConversationId, Message>,
    pub cursor: Option<String>,
    pub drafts: BTreeMap<ConversationId, Draft>,
    pub drafted: Vec<ConversationId>,
    pub persistent: bool,
    pub outgoing: BTreeMap<String, OutgoingOperation>,
    pub query: String,
    pub hits: Vec<MessageSearchHit>,
    pub more: bool,
    pub offset: u32,
    pub notice: String,
    pub send_notices: BTreeMap<ConversationId, String>,
    pub last_sent: BTreeMap<ConversationId, String>,
}

impl Model {
    pub fn label(&self, id: &ConversationId) -> String {
        self.conversations
            .iter()
            .find(|conversation| &conversation.id == id)
            .map(|conversation| {
                if let Some(title) = conversation
                    .title
                    .as_ref()
                    .filter(|title| !title.trim().is_empty())
                {
                    return title.clone();
                }
                let names = conversation
                    .participants
                    .iter()
                    .filter(|p| !p.is_self)
                    .filter_map(|p| p.display_name.clone().or(p.address.clone()))
                    .collect::<Vec<_>>()
                    .join(", ");
                if names.is_empty() {
                    "Conversation".into()
                } else {
                    names
                }
            })
            .unwrap_or_else(|| "Saved conversation".into())
    }
    pub fn selected_draft(&self) -> Option<&Draft> {
        self.selected.as_ref().and_then(|id| self.drafts.get(id))
    }
    pub fn edit(&mut self, id: ConversationId, text: String, reply_to: Option<MessageId>) {
        let draft = self
            .drafts
            .entry(id.clone())
            .or_insert_with(|| Draft::new(id));
        if draft.value.text != text || draft.value.reply_to != reply_to {
            draft.value.text = text;
            draft.value.reply_to = reply_to;
            draft.dirty = true;
            if !draft.conflict {
                draft.error.clear();
            }
        }
    }
    pub fn draft_loaded(&mut self, remote: MessageDraft, replace: bool) {
        let draft = self
            .drafts
            .entry(remote.conversation_id.clone())
            .or_insert_with(|| Draft::new(remote.conversation_id.clone()));
        if replace
            || !draft.dirty
            || (draft.value.text == remote.text && draft.value.reply_to == remote.reply_to)
        {
            draft.value = remote;
            draft.dirty = false;
            draft.conflict = false;
            draft.error.clear();
        } else if draft.value.revision != remote.revision {
            draft.conflict = true;
            draft.error="This draft changed in another window. Copy your text before reloading the saved draft.".into();
        } else {
            draft.conflict = false;
            draft.error.clear();
        }
        draft.loaded = true;
    }
    pub fn draft_saved(&mut self, remote: MessageDraft) {
        if let Some(draft) = self.drafts.get_mut(&remote.conversation_id) {
            draft.saving = false;
            draft.value.revision = remote.revision;
            draft.dirty =
                draft.value.text != remote.text || draft.value.reply_to != remote.reply_to;
            draft.error.clear();
            let id = remote.conversation_id;
            self.drafted.retain(|known| known != &id);
            if !remote.text.is_empty() || remote.reply_to.is_some() {
                self.drafted.push(id);
            }
        }
    }
    pub fn accepted(&mut self, sent: &MessageDraft) {
        if let Some(draft) = self.drafts.get_mut(&sent.conversation_id)
            && draft.value.text == sent.text
            && draft.value.reply_to == sent.reply_to
        {
            draft.value.text.clear();
            draft.value.reply_to = None;
            draft.dirty = true;
        }
    }
    pub fn can(&self, capability: MessagingCapability) -> bool {
        self.connected
            && self.selected.as_ref().is_some_and(|id| {
                self.accounts.iter().any(|account| {
                    account.id == id.account_id && account.connected && account.authenticated
                }) && self.conversations.iter().any(|conversation| {
                    conversation.id == *id && conversation.capabilities.contains(&capability)
                })
            })
    }
    fn remember_preview(&mut self, message: &Message) {
        let id = &message.id.conversation_id;
        if self.previews.get(id).is_none_or(|old| {
            old.id == message.id
                || (old.sent_at, &old.id.local_id) <= (message.sent_at, &message.id.local_id)
        }) {
            self.previews.insert(id.clone(), message.clone());
        }
        while self.previews.len() > 512 {
            let Some(oldest) = self
                .previews
                .iter()
                .min_by_key(|(_, message)| message.sent_at)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            self.previews.remove(&oldest);
        }
    }
    pub fn merge(&mut self, mut messages: Vec<Message>) {
        for message in messages.drain(..) {
            self.remember_preview(&message);
            if self.selected.as_ref() != Some(&message.id.conversation_id) {
                continue;
            }
            self.messages.retain(|old| old.id != message.id);
            self.messages.push(message);
        }
        self.messages
            .sort_by(|a, b| (a.sent_at, &a.id.local_id).cmp(&(b.sent_at, &b.id.local_id)));
        if self.messages.len() > 300 {
            if self.browsing_older {
                self.messages.truncate(300);
            } else {
                self.messages.drain(..self.messages.len() - 300);
            }
        }
    }
    pub fn state(&mut self, payload: ServerPayload) {
        match payload {
            ServerPayload::Snapshot {
                messaging_accounts,
                conversations,
                ..
            } => {
                self.accounts = messaging_accounts;
                self.conversations = conversations;
                self.previews.clear();
                self.outgoing.clear();
            }
            ServerPayload::AccountAdded { account } | ServerPayload::AccountUpdated { account } => {
                self.accounts.retain(|old| old.id != account.id);
                self.accounts.push(account);
            }
            ServerPayload::AccountRemoved { account_id } => {
                self.hits
                    .retain(|hit| hit.message_id.conversation_id.account_id != account_id);
                self.drafted.retain(|id| id.account_id != account_id);
                self.outgoing
                    .retain(|_, operation| operation.conversation_id.account_id != account_id);
                self.send_notices
                    .retain(|id, _| id.account_id != account_id);
                self.last_sent.retain(|id, _| id.account_id != account_id);
                self.previews.retain(|id, _| id.account_id != account_id);
                self.accounts.retain(|account| account.id != account_id);
                self.conversations
                    .retain(|conversation| conversation.id.account_id != account_id);
                self.drafts.retain(|id, _| id.account_id != account_id);
                if self
                    .selected
                    .as_ref()
                    .is_some_and(|id| id.account_id == account_id)
                {
                    self.selected = None;
                    self.messages.clear();
                }
            }
            ServerPayload::ConversationAdded { conversation }
            | ServerPayload::ConversationUpdated { conversation } => {
                self.conversations.retain(|old| old.id != conversation.id);
                self.conversations.push(conversation);
            }
            ServerPayload::ConversationRemoved { conversation_id } => {
                self.previews.remove(&conversation_id);
                self.conversations.retain(|c| c.id != conversation_id)
            }
            ServerPayload::MessageAdded { message } | ServerPayload::MessageUpdated { message } => {
                self.remember_preview(&message);
                if self.anchor.is_none() || self.messages.iter().any(|old| old.id == message.id) {
                    self.merge(vec![message]);
                }
            }
            ServerPayload::MessageRemoved { message_id } => {
                self.previews.retain(|_, message| message.id != message_id);
                self.messages.retain(|message| message.id != message_id)
            }
            ServerPayload::OutgoingOperations { operations, .. } => {
                for operation in operations {
                    self.outgoing.insert(operation.id.clone(), operation);
                }
            }
            ServerPayload::OutgoingOperation { operation } => {
                self.outgoing.insert(operation.id.clone(), operation);
            }
            ServerPayload::OutgoingOperationRemoved { operation_id } => {
                self.outgoing.remove(&operation_id);
            }
            _ => {}
        }
        while self.outgoing.len() > 512 {
            let oldest = self
                .outgoing
                .values()
                .min_by_key(|op| op.created_at)
                .unwrap()
                .id
                .clone();
            self.outgoing.remove(&oldest);
        }
    }
    pub fn outcome(&self) -> Option<&OutgoingOutcome> {
        self.outgoing
            .values()
            .filter(|operation| Some(&operation.conversation_id) == self.selected.as_ref())
            .max_by_key(|operation| operation.created_at)
            .map(|operation| &operation.outcome)
    }
}

pub fn outcome_label(outcome: &OutgoingOutcome) -> &'static str {
    match outcome {
        OutgoingOutcome::Submitting => "Submitting…",
        OutgoingOutcome::Unknown => "Outcome unknown. Check the conversation before sending again.",
        OutgoingOutcome::Rejected => "Send rejected",
        OutgoingOutcome::Provider(status) => match status {
            MessageStatus::Accepted => "Accepted; delivery is not confirmed",
            MessageStatus::Sent => "Sent",
            MessageStatus::Delivered => "Delivered",
            MessageStatus::Displayed => "Read",
            MessageStatus::Failed(_) => "Sending failed",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn draft(text: &str, revision: u64) -> MessageDraft {
        MessageDraft {
            conversation_id: ConversationId::new(MessagingAccountId::new("fixture"), "thread"),
            text: text.into(),
            reply_to: None,
            revision,
        }
    }
    #[test]
    fn acknowledgements_preserve_edits_made_while_saving_or_sending() {
        let mut model = Model::default();
        let first = draft("first", 1);
        model.draft_loaded(first.clone(), false);
        model.edit(first.conversation_id.clone(), "second".into(), None);
        model.draft_saved(first.clone());
        assert_eq!(model.drafts[&first.conversation_id].value.text, "second");
        assert!(model.drafts[&first.conversation_id].dirty);
        model.accepted(&first);
        assert_eq!(model.drafts[&first.conversation_id].value.text, "second");
        model.accepted(&draft("second", 1));
        assert!(model.drafts[&first.conversation_id].value.text.is_empty());
    }
    #[test]
    fn reconnect_reconciles_uncertain_saves_without_overwriting_another_window() {
        let mut model = Model::default();
        let id = draft("", 0).conversation_id;
        model.draft_loaded(draft("", 0), false);
        model.edit(id.clone(), "mine".into(), None);
        model.draft_loaded(draft("theirs", 3), false);
        assert!(model.drafts[&id].conflict);
        assert_eq!(model.drafts[&id].value.text, "mine");
        model.draft_loaded(draft("mine", 4), false);
        assert!(!model.drafts[&id].dirty);
        assert!(!model.drafts[&id].conflict);
    }
    #[test]
    fn previews_keep_newest_observed_text_and_forget_removed_content() {
        let conversation_id = draft("", 0).conversation_id;
        let mut message = Message {
            id: MessageId::new(conversation_id.clone(), "new"),
            sender: Participant {
                local_id: "peer".into(),
                display_name: None,
                address: None,
                is_self: false,
            },
            transport: None,
            sent_at: Some(200),
            text: Some("Latest observed".into()),
            attachments: vec![],
            reply_to: None,
            reactions: vec![],
            deleted: false,
        };
        let mut model = Model::default();
        model.state(ServerPayload::MessageAdded {
            message: message.clone(),
        });
        let mut older = message.clone();
        older.id.local_id = "old".into();
        older.sent_at = Some(100);
        older.text = Some("Older history".into());
        model.merge(vec![older]);
        assert_eq!(
            model.previews[&conversation_id].text.as_deref(),
            Some("Latest observed")
        );
        message.deleted = true;
        message.text = None;
        model.state(ServerPayload::MessageUpdated {
            message: message.clone(),
        });
        assert!(model.previews[&conversation_id].deleted);
        model.state(ServerPayload::MessageRemoved {
            message_id: message.id.clone(),
        });
        assert!(model.previews.is_empty());
        model.merge(vec![message]);
        model.state(ServerPayload::AccountRemoved {
            account_id: conversation_id.account_id,
        });
        assert!(model.previews.is_empty());
    }
}
