use handover_core::*;
use handover_ipc::ServerPayload;
use std::collections::{BTreeMap, BTreeSet};

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
                pending_operation: None,
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
    pub newer_gap: bool,
    pub messages: Vec<Message>,
    pub previews: BTreeMap<ConversationId, Message>,
    pub cursor: Option<String>,
    pub drafts: BTreeMap<ConversationId, Draft>,
    pub drafted: Vec<ConversationId>,
    pub persistent: bool,
    pub outgoing: BTreeMap<String, OutgoingOperation>,
    pub typing: BTreeMap<ConversationId, Vec<String>>,
    read_states: BTreeMap<ConversationId, ReadState>,
    pub statuses: BTreeMap<MessageId, MessageStatus>,
    pub new_messages: BTreeSet<MessageId>,
    pub new_messages_overflow: bool,
    pub query: String,
    pub hits: Vec<MessageSearchHit>,
    pub more: bool,
    pub offset: u32,
    pub notice: String,
    pub send_notices: BTreeMap<ConversationId, String>,
    pub last_sent: BTreeMap<ConversationId, String>,
}

impl Model {
    pub fn reset_read(&mut self, states: Vec<ReadState>) {
        self.read_states.clear();
        for state in states.into_iter().take(4096) {
            self.apply_read(state);
        }
    }
    fn apply_read(&mut self, state: ReadState) {
        if self
            .conversations
            .iter()
            .any(|conversation| conversation.id == state.conversation_id)
        {
            self.read_states
                .insert(state.conversation_id.clone(), state);
            while self.read_states.len() > 4096 {
                self.read_states.pop_first();
            }
        }
    }
    pub fn unread(&self, id: &ConversationId) -> Option<bool> {
        self.read_states
            .get(id)
            .map(|state| state.unread)
            .or_else(|| {
                self.conversations
                    .iter()
                    .find(|conversation| &conversation.id == id)
                    .and_then(|conversation| conversation.unread_count.map(|count| count > 0))
            })
    }

    pub fn unread_boundary(&self, id: &ConversationId) -> Option<UnreadBoundary> {
        if self.unread(id) != Some(true) {
            return None;
        }
        let conversation = self.conversations.iter().find(|c| &c.id == id)?;
        Some(UnreadBoundary {
            conversation_id: id.clone(),
            last_read_message_id: self
                .read_states
                .get(id)
                .and_then(|state| state.last_read_message_id.clone()),
            latest_message_id: conversation.latest_message_id.clone(),
            count: conversation.unread_count,
        })
    }

    pub fn reset_typing(&mut self, states: Vec<TypingState>) {
        self.typing.clear();
        for state in states.into_iter().take(512) {
            self.apply_typing(state);
        }
    }
    fn apply_typing(&mut self, state: TypingState) {
        if !self
            .conversations
            .iter()
            .any(|conversation| conversation.id == state.conversation_id)
        {
            return;
        }
        if !self.accounts.iter().any(|account| {
            account.id == state.conversation_id.account_id
                && account.connected
                && account.authenticated
        }) {
            return;
        }
        let participants = state
            .participant_ids
            .into_iter()
            .take(64)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if participants.is_empty() {
            self.typing.remove(&state.conversation_id);
        } else {
            self.typing.insert(state.conversation_id, participants);
        }
        while self.typing.len() > 512 {
            self.typing.pop_first();
        }
    }
    pub fn typing_label(&self) -> Option<String> {
        self.typing_label_for(self.selected.as_ref()?)
    }
    pub fn typing_label_for(&self, id: &ConversationId) -> Option<String> {
        if !self.connected
            || !self.accounts.iter().any(|account| {
                account.id == id.account_id && account.connected && account.authenticated
            })
        {
            return None;
        }
        let peers = self.typing.get(id)?;
        let conversation = self
            .conversations
            .iter()
            .find(|conversation| &conversation.id == id)?;
        let peers = peers
            .iter()
            .filter(|id| {
                !conversation
                    .participants
                    .iter()
                    .any(|peer| peer.is_self && &peer.local_id == *id)
            })
            .collect::<Vec<_>>();
        let first = peers.first()?;
        let name = conversation
            .participants
            .iter()
            .find(|peer| &peer.local_id == *first)
            .and_then(|peer| {
                peer.display_name
                    .as_deref()
                    .filter(|name| !name.trim().is_empty())
                    .or_else(|| {
                        peer.address
                            .as_deref()
                            .filter(|address| !address.trim().is_empty())
                    })
            })
            .unwrap_or("Someone");
        Some(if peers.len() == 1 {
            format!("{name} is typing…")
        } else if peers.len() == 2 {
            format!("{name} and one other are typing…")
        } else {
            format!("{name} and {} others are typing…", peers.len() - 1)
        })
    }
    pub fn has_own_reaction(&self, id: &MessageId, emoji: &str) -> bool {
        let Some(conversation) = self
            .conversations
            .iter()
            .find(|conversation| conversation.id == id.conversation_id)
        else {
            return false;
        };
        self.messages
            .iter()
            .find(|message| &message.id == id && !message.deleted)
            .is_some_and(|message| {
                message.reactions.iter().any(|reaction| {
                    reaction.emoji == emoji
                        && reaction.participant_ids.iter().any(|actor| {
                            conversation.participants.iter().any(|participant| {
                                participant.is_self && &participant.local_id == actor
                            })
                        })
                })
            })
    }

    pub fn message_status(&self, id: &MessageId) -> Option<&'static str> {
        if let Some(operation) = self.message_operation(id) {
            return Some(match &operation.outcome {
                OutgoingOutcome::Submitting => "Sending…",
                OutgoingOutcome::Unknown => "Outcome unknown",
                OutgoingOutcome::Rejected => "Rejected",
                OutgoingOutcome::Provider(status) => status_label(status),
            });
        }
        self.statuses.get(id).map(status_label)
    }
    fn message_operation(&self, id: &MessageId) -> Option<&OutgoingOperation> {
        self.outgoing
            .values()
            .filter(|operation| operation.message_id.as_ref() == Some(id))
            .max_by_key(|operation| operation.updated_at)
    }
    pub fn message_failure(&self, id: &MessageId) -> Option<SendFailure> {
        match self
            .message_operation(id)
            .map(|operation| &operation.outcome)
        {
            Some(OutgoingOutcome::Rejected) => Some(SendFailure::Rejected),
            Some(OutgoingOutcome::Provider(MessageStatus::Failed(reason))) => Some(*reason),
            Some(_) => None,
            None => match self.statuses.get(id) {
                Some(MessageStatus::Failed(reason)) => Some(*reason),
                _ => None,
            },
        }
    }
    pub fn send_preview(&self, operation: &str) -> &'static str {
        match self
            .outgoing
            .get(operation)
            .map(|operation| &operation.outcome)
        {
            Some(
                OutgoingOutcome::Submitting | OutgoingOutcome::Provider(MessageStatus::Accepted),
            ) => "Sending",
            Some(
                OutgoingOutcome::Rejected | OutgoingOutcome::Provider(MessageStatus::Failed(_)),
            ) => "Not sent",
            Some(OutgoingOutcome::Provider(
                MessageStatus::Sent | MessageStatus::Delivered | MessageStatus::Displayed,
            )) => "Sent",
            Some(OutgoingOutcome::Unknown) | None => "Check send",
        }
    }
    pub fn clear_new_messages(&mut self) {
        self.new_messages.clear();
        self.new_messages_overflow = false;
    }
    fn note_arrival(&mut self, message: &Message) {
        if !self.connected
            || message.sender.is_self
            || message.deleted
            || self.selected.as_ref() != Some(&message.id.conversation_id)
            || (!self.browsing_older && self.anchor.is_none())
            || self.messages.iter().any(|known| known.id == message.id)
        {
            return;
        }
        let newer = self
            .previews
            .get(&message.id.conversation_id)
            .is_some_and(|last| {
                last.id != message.id
                    && message
                        .sent_at
                        .zip(last.sent_at)
                        .is_some_and(|(new, old)| new > old)
            });
        if newer {
            if self.new_messages.len() < 300 {
                self.new_messages.insert(message.id.clone());
            } else {
                self.new_messages_overflow = true;
            }
        }
    }
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
        if self
            .drafts
            .get(&id)
            .is_some_and(|draft| draft.value.pending_operation.is_some())
        {
            return;
        }
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
        let id = remote.conversation_id.clone();
        let present = !remote.text.is_empty() || remote.reply_to.is_some();
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
        draft.saving = false;
        draft.loaded = true;
        self.drafted.retain(|known| known != &id);
        if present {
            self.drafted.push(id);
            self.drafted.sort();
        }
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
                self.drafted.sort();
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
    pub fn remember_preview(&mut self, message: &Message) {
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
                self.newer_gap = true;
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
                typing_states,
                read_states,
                ..
            } => {
                self.accounts = messaging_accounts;
                self.conversations = conversations;
                self.reset_typing(typing_states);
                self.reset_read(read_states);
                self.statuses.clear();
                self.clear_new_messages();
                self.previews.clear();
                self.outgoing.clear();
            }
            ServerPayload::AccountAdded { account } | ServerPayload::AccountUpdated { account } => {
                if !account.connected || !account.authenticated {
                    self.typing.retain(|id, _| id.account_id != account.id);
                }
                self.accounts.retain(|old| old.id != account.id);
                self.accounts.push(account);
            }
            ServerPayload::AccountRemoved { account_id } => {
                self.read_states.retain(|id, _| id.account_id != account_id);
                self.typing.retain(|id, _| id.account_id != account_id);
                self.statuses
                    .retain(|id, _| id.conversation_id.account_id != account_id);
                self.new_messages
                    .retain(|id| id.conversation_id.account_id != account_id);
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
                // A newer explicit unread count supersedes an earlier read event.
                // Unknown counts do not erase the last reported read state.
                if conversation.unread_count.is_some() {
                    self.read_states.remove(&conversation.id);
                }
                self.conversations.retain(|old| old.id != conversation.id);
                self.conversations.push(conversation);
            }
            ServerPayload::ConversationRemoved { conversation_id } => {
                self.read_states.remove(&conversation_id);
                self.typing.remove(&conversation_id);
                self.statuses
                    .retain(|id, _| id.conversation_id != conversation_id);
                self.new_messages
                    .retain(|id| id.conversation_id != conversation_id);
                self.previews.remove(&conversation_id);
                self.conversations.retain(|c| c.id != conversation_id)
            }
            ServerPayload::MessageAdded { message } => {
                self.note_arrival(&message);
                self.apply_message(message);
            }
            ServerPayload::MessageUpdated { message } => {
                self.apply_message(message);
            }
            ServerPayload::Typing { state } => self.apply_typing(state),
            ServerPayload::ReadState { state } => self.apply_read(state),
            ServerPayload::MessageStatus { update } => {
                if self
                    .accounts
                    .iter()
                    .any(|account| account.id == update.message_id.conversation_id.account_id)
                {
                    self.statuses.insert(update.message_id, update.status);
                }
                while self.statuses.len() > 512 {
                    self.statuses.pop_first();
                }
            }
            ServerPayload::MessageRemoved { message_id } => {
                self.statuses.remove(&message_id);
                self.new_messages.remove(&message_id);
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
    fn apply_message(&mut self, message: Message) {
        self.remember_preview(&message);
        if self.anchor.is_none() || self.messages.iter().any(|old| old.id == message.id) {
            self.merge(vec![message]);
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

#[derive(Clone, Debug)]
pub struct UnreadBoundary {
    pub conversation_id: ConversationId,
    pub last_read_message_id: Option<MessageId>,
    pub latest_message_id: Option<MessageId>,
    pub count: Option<u64>,
}

impl UnreadBoundary {
    pub fn first_unread(&self, messages: &[Message]) -> Option<MessageId> {
        let scoped = messages
            .iter()
            .filter(|message| message.id.conversation_id == self.conversation_id)
            .collect::<Vec<_>>();
        if let Some(anchor) = &self.last_read_message_id {
            if anchor.conversation_id != self.conversation_id {
                return None;
            }
            let position = scoped.iter().position(|message| message.id == *anchor)?;
            return scoped[position + 1..]
                .iter()
                .find(|message| !message.sender.is_self && !message.deleted)
                .map(|message| message.id.clone());
        }
        let count = self.count.filter(|count| *count > 0)?;
        let latest = self.latest_message_id.as_ref()?;
        if latest.conversation_id != self.conversation_id {
            return None;
        }
        let end = scoped.iter().position(|message| message.id == *latest)?;
        let incoming = scoped[..=end]
            .iter()
            .filter(|message| !message.sender.is_self && !message.deleted)
            .collect::<Vec<_>>();
        let index = incoming.len().checked_sub(usize::try_from(count).ok()?)?;
        Some(incoming[index].id.clone())
    }
}

fn status_label(status: &MessageStatus) -> &'static str {
    match status {
        MessageStatus::Accepted => "Accepted",
        MessageStatus::Sent => "Sent",
        MessageStatus::Delivered => "Delivered",
        MessageStatus::Displayed => "Read",
        MessageStatus::Failed(reason) => failure_label(*reason),
    }
}

pub fn failure_label(reason: SendFailure) -> &'static str {
    match reason {
        SendFailure::TooLarge => "Failed · File too large",
        SendFailure::NoRoute => "Failed · No route",
        SendFailure::Encryption => "Failed · Encryption error",
        SendFailure::Rejected => "Failed · Rejected",
        SendFailure::TimedOut => "Failed · Timed out",
        SendFailure::Disconnected => "Failed · Disconnected",
        SendFailure::Transport => "Failed",
    }
}

pub fn failure_detail(reason: SendFailure) -> &'static str {
    match reason {
        SendFailure::TooLarge => "The service reported that an attachment exceeds its size limit.",
        SendFailure::NoRoute => "The service reported that no sending route is available.",
        SendFailure::Encryption => "The service couldn't encrypt this message.",
        SendFailure::Rejected => "This send was rejected.",
        SendFailure::TimedOut => "The service reported that sending timed out.",
        SendFailure::Disconnected => "The service reported a disconnection while sending.",
        SendFailure::Transport => "The service reported a send failure without further detail.",
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
    fn message(id: &ConversationId, local_id: &str, is_self: bool, deleted: bool) -> Message {
        Message {
            id: MessageId::new(id.clone(), local_id),
            sender: Participant {
                local_id: if is_self { "self" } else { "peer" }.into(),
                display_name: None,
                address: None,
                is_self,
            },
            transport: None,
            sent_at: None,
            text: None,
            attachments: vec![],
            reply_preview: None,
            reply_to: None,
            reactions: vec![],
            deleted,
        }
    }
    fn boundary(
        id: &ConversationId,
        count: Option<u64>,
        latest: Option<&str>,
        anchor: Option<&str>,
    ) -> UnreadBoundary {
        UnreadBoundary {
            conversation_id: id.clone(),
            last_read_message_id: anchor.map(|v| MessageId::new(id.clone(), v)),
            latest_message_id: latest.map(|v| MessageId::new(id.clone(), v)),
            count,
        }
    }
    fn draft(text: &str, revision: u64) -> MessageDraft {
        MessageDraft {
            conversation_id: ConversationId::new(MessagingAccountId::new("fixture"), "thread"),
            text: text.into(),
            reply_to: None,
            revision,
            pending_operation: None,
        }
    }
    #[test]
    fn unknown_submission_does_not_offer_recovery_from_an_older_failed_status() {
        let id = draft("", 0).conversation_id;
        let message = MessageId::new(id.clone(), "message");
        let mut model = Model::default();
        model.statuses.insert(
            message.clone(),
            MessageStatus::Failed(SendFailure::TooLarge),
        );
        assert_eq!(model.message_failure(&message), Some(SendFailure::TooLarge));
        model.outgoing.insert(
            "send".into(),
            OutgoingOperation {
                id: "send".into(),
                conversation_id: id,
                kind: OutgoingOperationKind::Text,
                created_at: 1,
                updated_at: 2,
                outcome: OutgoingOutcome::Unknown,
                message_id: Some(message.clone()),
            },
        );
        assert_eq!(model.message_failure(&message), None);
        assert_eq!(model.send_preview("send"), "Check send");
        model.outgoing.get_mut("send").unwrap().outcome =
            OutgoingOutcome::Provider(MessageStatus::Failed(SendFailure::NoRoute));
        assert_eq!(model.message_failure(&message), Some(SendFailure::NoRoute));
        assert_eq!(model.send_preview("send"), "Not sent");
        assert_eq!(model.send_preview("missing"), "Check send");
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
            reply_preview: None,
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
    #[test]
    fn deep_history_stays_bounded_and_remembers_a_missing_newer_tail() {
        let id = draft("", 0).conversation_id;
        let mut model = Model {
            selected: Some(id.clone()),
            browsing_older: true,
            ..Default::default()
        };
        model.merge(
            (0..350)
                .map(|n| Message {
                    id: MessageId::new(id.clone(), format!("m{n:03}")),
                    sender: Participant {
                        local_id: "peer".into(),
                        display_name: None,
                        address: None,
                        is_self: false,
                    },
                    transport: None,
                    sent_at: Some(n),
                    text: Some("history".into()),
                    attachments: vec![],
                    reply_preview: None,
                    reply_to: None,
                    reactions: vec![],
                    deleted: false,
                })
                .collect(),
        );
        assert_eq!(model.messages.len(), 300);
        assert_eq!(model.messages.first().unwrap().id.local_id, "m000");
        assert_eq!(model.messages.last().unwrap().id.local_id, "m299");
        assert!(model.newer_gap);
    }

    #[test]
    fn unread_anchor_requires_loaded_anchor_and_ignores_out_of_scope_messages() {
        let id = draft("", 0).conversation_id;
        let other = ConversationId::new(id.account_id.clone(), "other");
        let b = boundary(&id, None, None, Some("read"));
        let messages = vec![
            message(&other, "foreign", false, false),
            message(&id, "read", false, false),
            message(&id, "next", false, false),
        ];
        assert_eq!(
            b.first_unread(&messages),
            Some(MessageId::new(id.clone(), "next"))
        );
        assert_eq!(b.first_unread(&messages[0..1]), None);
    }

    #[test]
    fn unread_count_is_snapshot_bounded_and_excludes_outgoing_or_deleted_messages() {
        let id = draft("", 0).conversation_id;
        let b = boundary(&id, Some(2), Some("latest"), None);
        let messages = vec![
            message(&id, "old", false, false),
            message(&id, "out", true, false),
            message(&id, "gone", false, true),
            message(&id, "unread1", false, false),
            message(&id, "latest", false, false),
            message(&id, "live", false, false),
        ];
        assert_eq!(
            b.first_unread(&messages),
            Some(MessageId::new(id.clone(), "unread1"))
        );
        assert_eq!(
            boundary(&id, Some(4), Some("latest"), None).first_unread(&messages),
            None
        );
        assert_eq!(
            boundary(&id, None, Some("latest"), None).first_unread(&messages),
            None
        );
        assert_eq!(
            boundary(&id, Some(1), Some("absent"), None).first_unread(&messages),
            None
        );
    }
}

#[cfg(test)]
mod feedback_tests;
