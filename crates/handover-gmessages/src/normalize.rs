//! Normalization: wire contract records into backend-independent core
//! types. Every record is validated; failures name the record by id only
//! and never echo bodies, names, or addresses.

use std::collections::BTreeSet;

use handover_core::{
    Attachment, Conversation, ConversationId, ConversationKind, Message, MessageId, MessageStatus,
    MessagingAccount, MessagingAccountId, MessagingCapability, Participant, Reaction, SendFailure,
    TransportKind, validate_account, validate_conversation, validate_message,
};

use crate::contract::{
    HelperEvent, WireConversation, WireConversationKind, WireMessage, WireTransport,
};

#[derive(Clone, Debug, PartialEq)]
pub enum NormalizeError {
    Invalid(String),
    UnknownStatus(String),
    UnknownConversationKind,
    UnsupportedCapability(String),
}

impl std::fmt::Display for NormalizeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(detail) => write!(formatter, "invalid helper record: {detail}"),
            Self::UnknownStatus(status) => {
                write!(formatter, "unknown message status {status:?}")
            }
            Self::UnknownConversationKind => {
                write!(formatter, "conversation kind not attested")
            }
            Self::UnsupportedCapability(name) => {
                write!(formatter, "unsupported capability {name:?}")
            }
        }
    }
}

impl std::error::Error for NormalizeError {}

impl From<handover_core::ValidationError> for NormalizeError {
    fn from(error: handover_core::ValidationError) -> Self {
        Self::Invalid(error.to_string())
    }
}

pub fn normalize_account(
    account: &str,
    label: &str,
    connected: bool,
    authenticated: bool,
) -> Result<MessagingAccount, NormalizeError> {
    let record = MessagingAccount {
        id: MessagingAccountId::new(account),
        label: label.into(),
        connected,
        authenticated,
    };
    validate_account(&record)?;
    Ok(record)
}

pub fn normalize_conversation(
    account: &MessagingAccountId,
    wire: WireConversation,
) -> Result<Conversation, NormalizeError> {
    let kind = match wire.kind {
        WireConversationKind::Direct => ConversationKind::Direct,
        WireConversationKind::Group => ConversationKind::Group,
        // Unknown shape is not representable: reject rather than guess.
        WireConversationKind::Unknown => return Err(NormalizeError::UnknownConversationKind),
    };
    let transport = match wire.transport {
        WireTransport::Rcs => TransportKind::Rcs,
        WireTransport::Sms => TransportKind::Sms,
        WireTransport::Mms => TransportKind::Mms,
        WireTransport::Unknown => TransportKind::Unknown,
    };
    let mut capabilities = BTreeSet::new();
    for name in &wire.capabilities {
        capabilities.insert(parse_capability(name)?);
    }
    let conversation = Conversation {
        id: ConversationId::new(account.clone(), wire.local_id),
        kind,
        transport,
        title: wire.title,
        participants: wire
            .participants
            .into_iter()
            .map(|participant| Participant {
                local_id: participant.local_id,
                display_name: participant.display_name,
                address: participant.address,
                is_self: participant.is_self,
            })
            .collect(),
        latest_message_id: wire.latest_message.map(|local_id| {
            MessageId::new(
                ConversationId::new(account.clone(), String::new()),
                local_id,
            )
        }),
        unread_count: wire.unread_count,
        cursor: wire.cursor,
        capabilities,
    };
    // Fix up the placeholder conversation scope on latest_message_id now
    // that the real id exists.
    let mut conversation = conversation;
    if let Some(latest) = conversation.latest_message_id.take() {
        conversation.latest_message_id =
            Some(MessageId::new(conversation.id.clone(), latest.local_id));
    }
    validate_conversation(&conversation)?;
    Ok(conversation)
}

pub fn parse_capability(name: &str) -> Result<MessagingCapability, NormalizeError> {
    match name {
        "text" => Ok(MessagingCapability::Text),
        "media" => Ok(MessagingCapability::Media),
        "reactions" => Ok(MessagingCapability::Reactions),
        "replies" => Ok(MessagingCapability::Replies),
        "read_receipts" => Ok(MessagingCapability::ReadReceipts),
        "typing_send" => Ok(MessagingCapability::TypingSend),
        "typing_receive" => Ok(MessagingCapability::TypingReceive),
        "group_create" => Ok(MessagingCapability::GroupCreate),
        "conversation_delete" => Ok(MessagingCapability::ConversationDelete),
        "message_delete_own" => Ok(MessagingCapability::MessageDeleteOwn),
        "attachment_download" => Ok(MessagingCapability::AttachmentDownload),
        // Closed attested set: unknown names are rejected, never promoted.
        // There is intentionally no edit/membership/disappearing name.
        other => Err(NormalizeError::UnsupportedCapability(other.into())),
    }
}

pub fn normalize_message(
    conversation_id: &ConversationId,
    wire: WireMessage,
) -> Result<Message, NormalizeError> {
    let message = Message {
        id: MessageId::new(conversation_id.clone(), wire.local_id),
        sender: Participant {
            local_id: wire.sender,
            display_name: None,
            address: None,
            is_self: false,
        },
        sent_at: wire.sent_at,
        text: wire.text,
        attachments: wire
            .attachments
            .into_iter()
            .map(|attachment| Attachment {
                local_id: attachment.local_id,
                mime: attachment.mime,
                name: attachment.name,
                size_bytes: attachment.size_bytes,
                staged_path: attachment.staged_path,
            })
            .collect(),
        reply_to: wire
            .reply_to
            .map(|local_id| MessageId::new(conversation_id.clone(), local_id)),
        reactions: wire
            .reactions
            .into_iter()
            .map(|reaction| Reaction {
                count: reaction.participant_ids.len() as u64,
                emoji: reaction.emoji,
                participant_ids: reaction.participant_ids,
            })
            .collect(),
        deleted: wire.deleted,
    };
    validate_message(&message)?;
    Ok(message)
}

/// Resolve sender display details from the conversation roster after
/// normalization. Unknown senders keep their opaque key with no invented
/// name; `is_self` comes from the roster entry when present.
pub fn resolve_sender(message: &mut Message, conversation: &Conversation) {
    if let Some(roster) = conversation
        .participants
        .iter()
        .find(|participant| participant.local_id == message.sender.local_id)
    {
        message.sender.display_name.clone_from(&roster.display_name);
        message.sender.address.clone_from(&roster.address);
        message.sender.is_self = roster.is_self;
    }
}

pub fn parse_status(status: &str) -> Result<MessageStatus, NormalizeError> {
    match status {
        "accepted" => Ok(MessageStatus::Accepted),
        "sent" => Ok(MessageStatus::Sent),
        "delivered" => Ok(MessageStatus::Delivered),
        "displayed" => Ok(MessageStatus::Displayed),
        "failed:too_large" => Ok(MessageStatus::Failed(SendFailure::TooLarge)),
        "failed:no_route" => Ok(MessageStatus::Failed(SendFailure::NoRoute)),
        "failed:encryption" => Ok(MessageStatus::Failed(SendFailure::Encryption)),
        "failed:rejected" => Ok(MessageStatus::Failed(SendFailure::Rejected)),
        "failed:timed_out" => Ok(MessageStatus::Failed(SendFailure::TimedOut)),
        "failed:disconnected" => Ok(MessageStatus::Failed(SendFailure::Disconnected)),
        "failed:transport" => Ok(MessageStatus::Failed(SendFailure::Transport)),
        // Never coerce an unknown token into a known state.
        other => Err(NormalizeError::UnknownStatus(other.into())),
    }
}

/// Extract ids from an event for size-bounded logging (no bodies).
pub fn event_ids(event: &HelperEvent) -> String {
    match event {
        HelperEvent::Hello { name, .. } => format!("hello name_len={}", name.len()),
        HelperEvent::Account { account, .. } => format!("account {account}"),
        HelperEvent::AccountRemoved { account } => format!("account_removed {account}"),
        HelperEvent::Pairing { account, .. } => format!("pairing {account}"),
        HelperEvent::Conversations {
            account,
            conversations,
            full,
        } => format!(
            "conversations {account} count={} full={full}",
            conversations.len()
        ),
        HelperEvent::ConversationRemoved {
            account,
            conversation,
        } => format!("conversation_removed {account}:{conversation}"),
        HelperEvent::Messages {
            account,
            conversation,
            messages,
            full,
            ..
        } => format!(
            "messages {account}:{conversation} count={} full={full}",
            messages.len()
        ),
        HelperEvent::MessageRemoved {
            account,
            conversation,
            message,
        } => format!("message_removed {account}:{conversation}:{message}"),
        HelperEvent::Status {
            account,
            conversation,
            message,
            status,
        } => format!("status {account}:{conversation}:{message} {status}"),
        HelperEvent::Typing {
            account,
            conversation,
            participants,
        } => format!(
            "typing {account}:{conversation} count={}",
            participants.len()
        ),
        HelperEvent::Read {
            account,
            conversation,
            ..
        } => format!("read {account}:{conversation}"),
        HelperEvent::CommandResult { request_id, ok, .. } => {
            format!("command_result {request_id} ok={ok}")
        }
        HelperEvent::Error { .. } => "error".into(),
    }
}
