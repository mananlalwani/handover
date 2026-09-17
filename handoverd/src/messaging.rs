//! Daemon-owned normalized messaging state.
//!
//! `handoverd` is authoritative: helper records are validated before they
//! enter these maps, outbound commands are validated against current state
//! and attested capabilities, and delivery outcome always arrives later as
//! a status event. Message windows are bounded per conversation; there is
//! no persistent full-history database.

use std::collections::{BTreeMap, VecDeque};

use handover_core::{
    Conversation, ConversationEvent, ConversationId, Message, MessageEvent, MessageId,
    MessageStatus, MessageStatusUpdate, MessagingAccount, MessagingAccountEvent,
    MessagingAccountId, MessagingCommand, MessagingEvent, ReadState, TypingState, ValidationError,
    validate_command, validate_conversation, validate_message,
};

/// Stored messages per conversation. Bounded: the daemon serves recent
/// windows and pages older history through the helper on demand.
pub(crate) const MAX_STORED_PER_CONVERSATION: usize = 300;
/// Maximum conversations tracked per account in memory.
pub(crate) const MAX_CONVERSATIONS_PER_ACCOUNT: usize = 500;

#[derive(Default)]
pub(crate) struct MessagingStore {
    accounts: BTreeMap<MessagingAccountId, MessagingAccount>,
    conversations: BTreeMap<ConversationId, Conversation>,
    messages: BTreeMap<ConversationId, VecDeque<Message>>,
    statuses: BTreeMap<MessageId, MessageStatus>,
    typing: BTreeMap<ConversationId, TypingState>,
    read: BTreeMap<ConversationId, ReadState>,
}

impl MessagingStore {
    pub(crate) fn apply(&mut self, event: MessagingEvent) -> MessagingOutcome {
        match event {
            MessagingEvent::Account(event) => self.apply_account(event),
            MessagingEvent::Conversation(event) => self.apply_conversation(event),
            MessagingEvent::Message(event) => self.apply_message(event),
            MessagingEvent::Status(update) => self.apply_status(update),
            MessagingEvent::Typing(state) => self.apply_typing(state),
            MessagingEvent::Read(state) => self.apply_read(state),
            // Transient prompt: visible to subscribers, never stored.
            MessagingEvent::Pairing(prompt) => {
                MessagingOutcome::changed(MessagingChange::Pairing(prompt.account_id))
            }
        }
    }

    /// Reconcile an authoritative window from the helper after (re)connect:
    /// upsert every record and drop locally stored messages the window no
    /// longer contains. Removals are reported so clients stay truthful.
    pub(crate) fn reconcile_window(
        &mut self,
        conversation_id: &ConversationId,
        messages: Vec<Message>,
    ) -> MessagingOutcome {
        let mut changes = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for message in messages {
            if validate_message(&message).is_err() {
                continue;
            }
            if message.id.conversation_id != *conversation_id {
                continue;
            }
            seen.insert(message.id.local_id.clone());
            let outcome = self.upsert_message(message);
            changes.extend(outcome.changes);
        }
        let window = self.messages.entry(conversation_id.clone()).or_default();
        let before: Vec<MessageId> = window.iter().map(|message| message.id.clone()).collect();
        window.retain(|message| seen.contains(&message.id.local_id));
        for id in before {
            if !seen.contains(&id.local_id) {
                self.statuses.remove(&id);
                changes.push(MessagingChange::MessageRemoved(id));
            }
        }
        // Rebuild order by (sent_at, local_id) so reconciled windows read
        // oldest-first regardless of arrival order.
        let window = self.messages.entry(conversation_id.clone()).or_default();
        let mut sorted: Vec<Message> = window.drain(..).collect();
        sorted.sort_by(|a, b| (a.sent_at, &a.id.local_id).cmp(&(b.sent_at, &b.id.local_id)));
        *window = sorted.into_iter().collect();
        MessagingOutcome {
            changed: !changes.is_empty(),
            changes,
        }
    }

    pub(crate) fn snapshot_accounts(&self) -> Vec<MessagingAccount> {
        self.accounts.values().cloned().collect()
    }

    pub(crate) fn snapshot_conversations(&self) -> Vec<Conversation> {
        self.conversations.values().cloned().collect()
    }

    pub(crate) fn snapshot_typing(&self) -> Vec<TypingState> {
        self.typing.values().cloned().collect()
    }

    pub(crate) fn snapshot_read(&self) -> Vec<ReadState> {
        self.read.values().cloned().collect()
    }

    pub(crate) fn account(&self, id: &MessagingAccountId) -> Option<&MessagingAccount> {
        self.accounts.get(id)
    }

    pub(crate) fn conversation(&self, id: &ConversationId) -> Option<&Conversation> {
        self.conversations.get(id)
    }

    pub(crate) fn message(&self, id: &MessageId) -> Option<&Message> {
        self.messages
            .get(&id.conversation_id)
            .and_then(|window| window.iter().find(|message| message.id == *id))
    }

    #[cfg(test)]
    pub(crate) fn status(&self, id: &MessageId) -> Option<&MessageStatus> {
        self.statuses.get(id)
    }

    /// Serve the local window newest-last, honoring an opaque cursor.
    /// `cursor` is a message local id: the page ends before it. Returns the
    /// page and the cursor for the next older page (`None` at the start).
    /// `Err(HistoryGap)` means the cursor is not in the stored window: the
    /// caller should page through the helper.
    pub(crate) fn history(
        &self,
        conversation_id: &ConversationId,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<(Vec<Message>, Option<String>), HistoryGap> {
        let window = self
            .messages
            .get(conversation_id)
            .ok_or(HistoryGap::UnknownConversation)?;
        let end = match cursor {
            None => window.len(),
            Some(cursor) => window
                .iter()
                .position(|message| message.id.local_id == cursor)
                .ok_or(HistoryGap::CursorOutsideWindow)?,
        };
        let start = end.saturating_sub(limit.max(1));
        let page: Vec<Message> = window
            .iter()
            .skip(start)
            .take(end - start)
            .cloned()
            .collect();
        let next = if start > 0 {
            page.first().map(|message| message.id.local_id.clone())
        } else {
            None
        };
        Ok((page, next))
    }

    pub(crate) fn validate_messaging_command(
        &self,
        command: &MessagingCommand,
    ) -> Result<(), MessagingValidationError> {
        match command {
            MessagingCommand::SendText {
                conversation_id, ..
            }
            | MessagingCommand::SendMedia {
                conversation_id, ..
            }
            | MessagingCommand::MarkRead {
                conversation_id, ..
            }
            | MessagingCommand::TypingStart {
                conversation_id, ..
            } => {
                let conversation = self
                    .conversations
                    .get(conversation_id)
                    .ok_or(MessagingValidationError::UnknownConversation)?;
                let account = self
                    .accounts
                    .get(&conversation.id.account_id)
                    .ok_or(MessagingValidationError::AccountUnavailable)?;
                if !account.connected || !account.authenticated {
                    return Err(MessagingValidationError::AccountUnavailable);
                }
                validate_command(command, Some(conversation), None)?;
                Ok(())
            }
            MessagingCommand::React { message_id, .. }
            | MessagingCommand::Unreact { message_id, .. }
            | MessagingCommand::DeleteMessage { message_id } => {
                let message = self
                    .message(message_id)
                    .ok_or(MessagingValidationError::UnknownMessage)?;
                let conversation = self
                    .conversations
                    .get(&message.id.conversation_id)
                    .ok_or(MessagingValidationError::UnknownConversation)?;
                validate_command(command, Some(conversation), Some(message))?;
                Ok(())
            }
            MessagingCommand::OpenConversation { account_id, .. } => {
                let account = self
                    .accounts
                    .get(account_id)
                    .ok_or(MessagingValidationError::UnknownAccount)?;
                if !account.connected || !account.authenticated {
                    return Err(MessagingValidationError::AccountUnavailable);
                }
                validate_command(command, None, None)?;
                Ok(())
            }
        }
        .map_err(MessagingValidationError::Invalid)
    }

    /// Remove every record owned by one account (logout/revoke).
    pub(crate) fn remove_account(&mut self, id: &MessagingAccountId) -> MessagingOutcome {
        let mut changes = Vec::new();
        let conversations: Vec<ConversationId> = self
            .conversations
            .keys()
            .filter(|conversation| &conversation.account_id == id)
            .cloned()
            .collect();
        for conversation in conversations {
            changes.extend(self.remove_conversation_inner(&conversation));
        }
        if self.accounts.remove(id).is_some() {
            changes.push(MessagingChange::AccountRemoved(id.clone()));
        }
        MessagingOutcome {
            changed: !changes.is_empty(),
            changes,
        }
    }

    fn apply_account(&mut self, event: MessagingAccountEvent) -> MessagingOutcome {
        match event {
            MessagingAccountEvent::Added(account) | MessagingAccountEvent::Updated(account) => {
                if self.accounts.get(&account.id) == Some(&account) {
                    return MessagingOutcome::unchanged();
                }
                let change = if self.accounts.contains_key(&account.id) {
                    MessagingChange::AccountUpdated(account.id.clone())
                } else {
                    MessagingChange::AccountAdded(account.id.clone())
                };
                self.accounts.insert(account.id.clone(), account);
                MessagingOutcome::changed(change)
            }
            MessagingAccountEvent::Removed(id) => self.remove_account(&id),
        }
    }

    fn apply_conversation(&mut self, event: ConversationEvent) -> MessagingOutcome {
        match event {
            ConversationEvent::Added(conversation) | ConversationEvent::Updated(conversation) => {
                if validate_conversation(&conversation).is_err() {
                    return MessagingOutcome::unchanged();
                }
                if !self.accounts.contains_key(&conversation.id.account_id) {
                    return MessagingOutcome::unchanged();
                }
                let count = self
                    .conversations
                    .keys()
                    .filter(|id| id.account_id == conversation.id.account_id)
                    .count();
                if !self.conversations.contains_key(&conversation.id)
                    && count >= MAX_CONVERSATIONS_PER_ACCOUNT
                {
                    return MessagingOutcome::unchanged();
                }
                let change = match self.conversations.get(&conversation.id) {
                    None => MessagingChange::ConversationAdded(conversation.id.clone()),
                    Some(previous) if previous == &conversation => {
                        return MessagingOutcome::unchanged();
                    }
                    Some(_) => MessagingChange::ConversationUpdated(conversation.id.clone()),
                };
                self.conversations
                    .insert(conversation.id.clone(), conversation);
                MessagingOutcome::changed(change)
            }
            ConversationEvent::Removed(id) => {
                let changes = self.remove_conversation_inner(&id);
                MessagingOutcome {
                    changed: !changes.is_empty(),
                    changes,
                }
            }
        }
    }

    fn remove_conversation_inner(&mut self, id: &ConversationId) -> Vec<MessagingChange> {
        let mut changes = Vec::new();
        if let Some(window) = self.messages.remove(id) {
            for message in window {
                self.statuses.remove(&message.id);
            }
        }
        self.typing.remove(id);
        self.read.remove(id);
        if self.conversations.remove(id).is_some() {
            changes.push(MessagingChange::ConversationRemoved(id.clone()));
        }
        changes
    }

    fn apply_message(&mut self, event: MessageEvent) -> MessagingOutcome {
        match event {
            MessageEvent::Added(message) | MessageEvent::Updated(message) => {
                self.upsert_message(message)
            }
            MessageEvent::Removed(id) => {
                let removed = self
                    .messages
                    .get_mut(&id.conversation_id)
                    .map(|window| {
                        let before = window.len();
                        window.retain(|message| message.id != id);
                        before != window.len()
                    })
                    .unwrap_or(false);
                if removed {
                    self.statuses.remove(&id);
                    MessagingOutcome::changed(MessagingChange::MessageRemoved(id))
                } else {
                    MessagingOutcome::unchanged()
                }
            }
        }
    }

    fn upsert_message(&mut self, message: Message) -> MessagingOutcome {
        if validate_message(&message).is_err() {
            return MessagingOutcome::unchanged();
        }
        let Some(conversation) = self.conversations.get(&message.id.conversation_id) else {
            return MessagingOutcome::unchanged();
        };
        let mut message = message;
        // Resolve sender display details from the roster; unknown senders
        // keep their opaque key with no invented name.
        if let Some(roster) = conversation
            .participants
            .iter()
            .find(|participant| participant.local_id == message.sender.local_id)
        {
            message.sender.display_name.clone_from(&roster.display_name);
            message.sender.address.clone_from(&roster.address);
            message.sender.is_self = roster.is_self;
        }
        let window = self
            .messages
            .entry(message.id.conversation_id.clone())
            .or_default();
        let change = match window.iter().find(|known| known.id == message.id) {
            None => MessagingChange::MessageAdded(message.id.clone()),
            Some(previous) if previous == &message => return MessagingOutcome::unchanged(),
            Some(_) => MessagingChange::MessageUpdated(message.id.clone()),
        };
        if let Some(existing) = window.iter_mut().find(|known| known.id == message.id) {
            *existing = message;
        } else {
            window.push_back(message);
            while window.len() > MAX_STORED_PER_CONVERSATION {
                if let Some(evicted) = window.pop_front() {
                    self.statuses.remove(&evicted.id);
                }
            }
        }
        // Keep the window oldest-first for stable paging.
        let mut sorted: Vec<Message> = window.drain(..).collect();
        sorted.sort_by(|a, b| (a.sent_at, &a.id.local_id).cmp(&(b.sent_at, &b.id.local_id)));
        *window = sorted.into_iter().collect();
        MessagingOutcome::changed(change)
    }

    fn apply_status(&mut self, update: MessageStatusUpdate) -> MessagingOutcome {
        if self.message(&update.message_id).is_none() {
            return MessagingOutcome::unchanged();
        }
        // Status is monotonic: Accepted < Sent < Delivered < Displayed, and
        // Failed is terminal. Backward moves and anything after Failed are
        // ignored rather than surfaced as regressions.
        let rank = status_rank(&update.status);
        if let Some(current) = self.statuses.get(&update.message_id) {
            if status_rank(current) == FAILED_RANK || rank < status_rank(current) {
                return MessagingOutcome::unchanged();
            }
            if current == &update.status {
                return MessagingOutcome::unchanged();
            }
        }
        self.statuses
            .insert(update.message_id.clone(), update.status.clone());
        MessagingOutcome::changed(MessagingChange::Status(update))
    }

    fn apply_typing(&mut self, state: TypingState) -> MessagingOutcome {
        if !self.conversations.contains_key(&state.conversation_id) {
            return MessagingOutcome::unchanged();
        }
        if state.participant_ids.is_empty() {
            if self.typing.remove(&state.conversation_id).is_some() {
                return MessagingOutcome::changed(MessagingChange::TypingCleared(
                    state.conversation_id,
                ));
            }
            return MessagingOutcome::unchanged();
        }
        if self.typing.get(&state.conversation_id) == Some(&state) {
            return MessagingOutcome::unchanged();
        }
        let id = state.conversation_id.clone();
        self.typing.insert(id.clone(), state);
        MessagingOutcome::changed(MessagingChange::Typing(id))
    }

    fn apply_read(&mut self, state: ReadState) -> MessagingOutcome {
        if !self.conversations.contains_key(&state.conversation_id) {
            return MessagingOutcome::unchanged();
        }
        if self.read.get(&state.conversation_id) == Some(&state) {
            return MessagingOutcome::unchanged();
        }
        let id = state.conversation_id.clone();
        self.read.insert(id.clone(), state);
        MessagingOutcome::changed(MessagingChange::Read(id))
    }
}

const FAILED_RANK: u8 = 99;

fn status_rank(status: &MessageStatus) -> u8 {
    match status {
        MessageStatus::Accepted => 0,
        MessageStatus::Sent => 1,
        MessageStatus::Delivered => 2,
        MessageStatus::Displayed => 3,
        MessageStatus::Failed(_) => FAILED_RANK,
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum HistoryGap {
    UnknownConversation,
    CursorOutsideWindow,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MessagingValidationError {
    Invalid(ValidationError),
    UnknownAccount,
    UnknownConversation,
    UnknownMessage,
    AccountUnavailable,
}

impl From<ValidationError> for MessagingValidationError {
    fn from(error: ValidationError) -> Self {
        Self::Invalid(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MessagingChange {
    AccountAdded(MessagingAccountId),
    AccountUpdated(MessagingAccountId),
    AccountRemoved(MessagingAccountId),
    ConversationAdded(ConversationId),
    ConversationUpdated(ConversationId),
    ConversationRemoved(ConversationId),
    MessageAdded(MessageId),
    MessageUpdated(MessageId),
    MessageRemoved(MessageId),
    Status(MessageStatusUpdate),
    Typing(ConversationId),
    TypingCleared(ConversationId),
    Read(ConversationId),
    Pairing(MessagingAccountId),
}

pub(crate) struct MessagingOutcome {
    pub(crate) changed: bool,
    pub(crate) changes: Vec<MessagingChange>,
}

impl MessagingOutcome {
    fn unchanged() -> Self {
        Self {
            changed: false,
            changes: Vec::new(),
        }
    }

    fn changed(change: MessagingChange) -> Self {
        Self {
            changed: true,
            changes: vec![change],
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use handover_core::{
        ConversationKind, MessagingCapability, Participant, SendFailure, TransportKind,
    };

    use super::*;

    fn account_id() -> MessagingAccountId {
        MessagingAccountId::new("gmessages:test")
    }

    fn account() -> MessagingAccount {
        MessagingAccount {
            id: account_id(),
            label: "Test".into(),
            connected: true,
            authenticated: true,
        }
    }

    fn conversation_id() -> ConversationId {
        ConversationId::new(account_id(), "thread-1")
    }

    fn participant(local_id: &str, is_self: bool) -> Participant {
        Participant {
            local_id: local_id.into(),
            display_name: Some(format!("Name {local_id}")),
            address: Some(format!("+1555000{local_id}")),
            is_self,
        }
    }

    fn conversation() -> Conversation {
        Conversation {
            id: conversation_id(),
            kind: ConversationKind::Direct,
            transport: TransportKind::Rcs,
            title: None,
            participants: vec![participant("self", true), participant("peer", false)],
            latest_message_id: None,
            unread_count: None,
            cursor: None,
            capabilities: BTreeSet::from([
                MessagingCapability::Text,
                MessagingCapability::Reactions,
                MessagingCapability::ReadReceipts,
                MessagingCapability::TypingSend,
                MessagingCapability::MessageDeleteOwn,
            ]),
        }
    }

    fn message(local_id: &str, sender: &str, sent_at: i64, text: &str) -> Message {
        Message {
            id: MessageId::new(conversation_id(), local_id),
            sender: Participant {
                local_id: sender.into(),
                display_name: None,
                address: None,
                is_self: false,
            },
            sent_at: Some(sent_at),
            text: Some(text.into()),
            attachments: vec![],
            reply_to: None,
            reactions: vec![],
            deleted: false,
        }
    }

    fn live_store() -> MessagingStore {
        let mut store = MessagingStore::default();
        store.apply(MessagingEvent::Account(MessagingAccountEvent::Added(
            account(),
        )));
        store.apply(MessagingEvent::Conversation(ConversationEvent::Added(
            conversation(),
        )));
        store
    }

    #[test]
    fn messages_require_known_conversation_and_deduplicate() {
        let mut store = MessagingStore::default();
        let first = message("m1", "peer", 10, "hello");
        assert!(
            !store
                .apply(MessagingEvent::Message(MessageEvent::Added(first.clone())))
                .changed,
            "unknown conversation must be dropped"
        );
        let mut store = live_store();
        assert!(
            store
                .apply(MessagingEvent::Message(MessageEvent::Added(first.clone())))
                .changed
        );
        // Identical re-delivery is a duplicate, not an update.
        assert!(
            !store
                .apply(MessagingEvent::Message(MessageEvent::Added(first.clone())))
                .changed
        );
        let mut edited = first.clone();
        edited.text = Some("hello!".into());
        let outcome = store.apply(MessagingEvent::Message(MessageEvent::Added(edited)));
        assert!(outcome.changed);
        assert!(matches!(
            outcome.changes.as_slice(),
            [MessagingChange::MessageUpdated(_)]
        ));
        // Sender display details resolve from the roster.
        let stored = store
            .message(&MessageId::new(conversation_id(), "m1"))
            .expect("stored");
        assert_eq!(stored.sender.display_name.as_deref(), Some("Name peer"));
        assert!(!stored.sender.is_self);
    }

    #[test]
    fn history_pages_newest_last_with_older_cursor() {
        let mut store = live_store();
        for (index, id) in ["m1", "m2", "m3", "m4"].iter().enumerate() {
            store.apply(MessagingEvent::Message(MessageEvent::Added(message(
                id,
                "peer",
                10 + index as i64,
                id,
            ))));
        }
        let (page, next) = store.history(&conversation_id(), 2, None).expect("page");
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].id.local_id, "m3");
        assert_eq!(page[1].id.local_id, "m4");
        assert_eq!(next.as_deref(), Some("m3"));
        let (older, end) = store
            .history(&conversation_id(), 10, next.as_deref())
            .expect("older page");
        assert_eq!(older.len(), 2);
        assert_eq!(older[0].id.local_id, "m1");
        assert!(end.is_none());
        assert_eq!(
            store.history(&conversation_id(), 2, Some("missing")),
            Err(HistoryGap::CursorOutsideWindow)
        );
    }

    #[test]
    fn status_moves_forward_and_failed_is_terminal() {
        let mut store = live_store();
        let id = MessageId::new(conversation_id(), "m1");
        store.apply(MessagingEvent::Message(MessageEvent::Added(message(
            "m1", "self", 10, "out",
        ))));
        for status in [
            MessageStatus::Accepted,
            MessageStatus::Sent,
            MessageStatus::Delivered,
            MessageStatus::Displayed,
        ] {
            assert!(
                store
                    .apply(MessagingEvent::Status(MessageStatusUpdate {
                        message_id: id.clone(),
                        status,
                    }))
                    .changed
            );
        }
        // Backward moves are ignored.
        assert!(
            !store
                .apply(MessagingEvent::Status(MessageStatusUpdate {
                    message_id: id.clone(),
                    status: MessageStatus::Sent,
                }))
                .changed
        );
        assert_eq!(store.status(&id), Some(&MessageStatus::Displayed));

        // Failed is terminal: nothing moves after it.
        let failing = MessageId::new(conversation_id(), "m2");
        store.apply(MessagingEvent::Message(MessageEvent::Added(message(
            "m2", "self", 11, "out",
        ))));
        assert!(
            store
                .apply(MessagingEvent::Status(MessageStatusUpdate {
                    message_id: failing.clone(),
                    status: MessageStatus::Failed(SendFailure::Transport),
                }))
                .changed
        );
        assert!(
            !store
                .apply(MessagingEvent::Status(MessageStatusUpdate {
                    message_id: failing.clone(),
                    status: MessageStatus::Displayed,
                }))
                .changed
        );
        // Unknown messages never gain status.
        assert!(
            !store
                .apply(MessagingEvent::Status(MessageStatusUpdate {
                    message_id: MessageId::new(conversation_id(), "ghost"),
                    status: MessageStatus::Accepted,
                }))
                .changed
        );
    }

    #[test]
    fn reconcile_replaces_window_and_reports_removals() {
        let mut store = live_store();
        for id in ["m1", "m2", "m3"] {
            store.apply(MessagingEvent::Message(MessageEvent::Added(message(
                id, "peer", 10, id,
            ))));
        }
        let outcome = store.reconcile_window(
            &conversation_id(),
            vec![
                message("m2", "peer", 11, "m2"),
                message("m4", "peer", 13, "m4"),
            ],
        );
        assert!(outcome.changed);
        assert!(outcome.changes.iter().any(
            |change| matches!(change, MessagingChange::MessageRemoved(id) if id.local_id == "m1")
        ));
        let (page, _) = store.history(&conversation_id(), 10, None).expect("page");
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].id.local_id, "m2");
    }

    #[test]
    fn account_removal_cascades_without_touching_other_accounts() {
        let mut store = live_store();
        let other = MessagingAccountId::new("gmessages:other");
        store.apply(MessagingEvent::Account(MessagingAccountEvent::Added(
            MessagingAccount {
                id: other.clone(),
                label: "Other".into(),
                connected: true,
                authenticated: true,
            },
        )));
        store.apply(MessagingEvent::Message(MessageEvent::Added(message(
            "m1", "peer", 10, "hello",
        ))));
        store.apply(MessagingEvent::Typing(TypingState {
            conversation_id: conversation_id(),
            participant_ids: vec!["peer".into()],
        }));
        let outcome = store.remove_account(&account_id());
        assert!(outcome.changed);
        assert!(store.account(&account_id()).is_none());
        assert!(store.conversation(&conversation_id()).is_none());
        assert!(store.snapshot_typing().is_empty());
        assert!(store.account(&other).is_some());
    }

    #[test]
    fn commands_validate_against_state_and_capabilities() {
        let store = live_store();
        assert!(
            store
                .validate_messaging_command(&MessagingCommand::SendText {
                    conversation_id: conversation_id(),
                    text: "hi".into(),
                })
                .is_ok()
        );
        // Media is not attested on this conversation.
        assert!(matches!(
            store.validate_messaging_command(&MessagingCommand::SendMedia {
                conversation_id: conversation_id(),
                file_url: "file:///tmp/x.bin".into(),
                caption: None,
            }),
            Err(MessagingValidationError::Invalid(
                ValidationError::UnsupportedCapability(_)
            ))
        ));
        assert!(matches!(
            store.validate_messaging_command(&MessagingCommand::MarkRead {
                conversation_id: ConversationId::new(account_id(), "missing"),
                message_id: None,
            }),
            Err(MessagingValidationError::UnknownConversation)
        ));
        // Offline accounts cannot send.
        let mut offline = live_store();
        let mut down = account();
        down.connected = false;
        offline.apply(MessagingEvent::Account(MessagingAccountEvent::Updated(
            down,
        )));
        assert_eq!(
            offline.validate_messaging_command(&MessagingCommand::SendText {
                conversation_id: conversation_id(),
                text: "hi".into(),
            }),
            Err(MessagingValidationError::AccountUnavailable)
        );
    }

    #[test]
    fn typing_clears_on_empty_and_read_tracks_state() {
        let mut store = live_store();
        assert!(
            store
                .apply(MessagingEvent::Typing(TypingState {
                    conversation_id: conversation_id(),
                    participant_ids: vec!["peer".into()],
                }))
                .changed
        );
        assert_eq!(store.snapshot_typing().len(), 1);
        let outcome = store.apply(MessagingEvent::Typing(TypingState {
            conversation_id: conversation_id(),
            participant_ids: vec![],
        }));
        assert!(outcome.changed);
        assert!(matches!(
            outcome.changes.as_slice(),
            [MessagingChange::TypingCleared(_)]
        ));
        assert!(
            store
                .apply(MessagingEvent::Read(ReadState {
                    conversation_id: conversation_id(),
                    last_read_message_id: Some(MessageId::new(conversation_id(), "m1")),
                    unread: false,
                }))
                .changed
        );
        assert_eq!(store.snapshot_read().len(), 1);
    }
}
