//! Backend-independent messaging domain types.
//!
//! Conversations belong to a [`MessagingAccountId`] (a messaging endpoint
//! such as one Google account's linked-device session), never to a physical
//! [`crate::DeviceId`]. Every backend-local identifier below the account
//! scope is opaque to clients: adapters map their own thread, message, and
//! sender keys into `local_id` fields and must never leak protocol enums,
//! status text, or transport quirks through this model.
//!
//! Capabilities are explicitly attested: a capability present in a
//! conversation's set was reported by the backend for that conversation.
//! Absence means "not attested", never "assumed absent". There is no edit,
//! membership-change, or disappearing-message capability because no backend
//! attests a first-class operation for those today.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Bounds enforced during normalization. Adapters must reject values
/// outside these bounds instead of truncating or guessing.
pub const MAX_TEXT_CHARS: usize = 8_000;
pub const MAX_ID_LEN: usize = 256;
pub const MAX_TITLE_CHARS: usize = 256;
pub const MAX_PARTICIPANTS: usize = 256;
pub const MAX_ATTACHMENTS_PER_MESSAGE: usize = 16;
pub const MAX_REACTIONS_PER_MESSAGE: usize = 32;
pub const MAX_REACTORS_PER_REACTION: usize = 256;
pub const MAX_EMOJI_CHARS: usize = 16;
pub const MAX_PAGE_LIMIT: u32 = 100;

/// A stable identifier for one messaging endpoint (for example one linked
/// Google Messages session). Namespaced by the owning backend, e.g.
/// `gmessages:<local>`.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct MessagingAccountId(String);

impl MessagingAccountId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MessagingAccountId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Normalized state for one messaging endpoint known to Handover.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MessagingAccount {
    pub id: MessagingAccountId,
    pub label: String,
    pub connected: bool,
    pub authenticated: bool,
}

/// A change to the set of messaging accounts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum MessagingAccountEvent {
    Added(MessagingAccount),
    Updated(MessagingAccount),
    Removed(MessagingAccountId),
}

/// A stable thread identity scoped to its messaging account. `local_id` is
/// the backend-local thread key and is opaque above the adapter.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ConversationId {
    pub account_id: MessagingAccountId,
    pub local_id: String,
}

impl ConversationId {
    pub fn new(account_id: MessagingAccountId, local_id: impl Into<String>) -> Self {
        Self {
            account_id,
            local_id: local_id.into(),
        }
    }
}

impl fmt::Display for ConversationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.account_id, self.local_id)
    }
}

/// One human or endpoint participant. `local_id` is the backend-local sender
/// key, opaque above the adapter. Display names are hints, never identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Participant {
    pub local_id: String,
    pub display_name: Option<String>,
    pub address: Option<String>,
    pub is_self: bool,
}

/// Thread shape. RCS-first ordering is deliberate; nothing here assumes SMS.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationKind {
    Direct,
    Group,
}

/// Which network family currently carries a conversation, where attested.
/// `Unknown` stays unknown rather than guessing SMS vs RCS.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Rcs,
    Sms,
    Mms,
    Unknown,
}

/// One conversation view. `cursor` is an opaque backend pagination token for
/// older history. `unread_count` is `None` when the backend cannot attest it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Conversation {
    pub id: ConversationId,
    pub kind: ConversationKind,
    pub transport: TransportKind,
    pub title: Option<String>,
    pub participants: Vec<Participant>,
    pub latest_message_id: Option<MessageId>,
    pub last_activity_at: Option<i64>,
    pub unread_count: Option<u64>,
    pub cursor: Option<String>,
    pub capabilities: BTreeSet<MessagingCapability>,
}

/// A change to the daemon's current set of conversations.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ConversationEvent {
    Added(Conversation),
    Updated(Conversation),
    Removed(ConversationId),
}

/// A stable message identity inside its conversation. `local_id` is the
/// backend-local message key, opaque above the adapter.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct MessageId {
    pub conversation_id: ConversationId,
    pub local_id: String,
}

impl MessageId {
    pub fn new(conversation_id: ConversationId, local_id: impl Into<String>) -> Self {
        Self {
            conversation_id,
            local_id: local_id.into(),
        }
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.conversation_id, self.local_id)
    }
}

/// What a message carries. Text and attachments may coexist (MMS/caption
/// case). There is deliberately no edit history: no backend attests a real
/// first-class edit operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Message {
    pub id: MessageId,
    pub sender: Participant,
    pub transport: Option<TransportKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<MessageId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<Reaction>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
}

/// A change to the daemon's current message window.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum MessageEvent {
    Added(Message),
    Updated(Message),
    Removed(MessageId),
}

/// One media/file part. Payload bytes never ride the normalized model; the
/// backend stages bytes out-of-band and hands over a path handle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Attachment {
    pub local_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged_path: Option<String>,
}

/// Coarse attachment family derived from the MIME type. Used for display
/// grouping only; the original MIME string remains on the attachment.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Image,
    Video,
    Audio,
    Document,
    Other,
}

impl AttachmentKind {
    pub fn of_mime(mime: Option<&str>) -> Self {
        let Some(mime) = mime else {
            return Self::Other;
        };
        let primary = mime.split(';').next().unwrap_or("").trim().to_lowercase();
        if primary.starts_with("image/") {
            Self::Image
        } else if primary.starts_with("video/") {
            Self::Video
        } else if primary.starts_with("audio/") {
            Self::Audio
        } else if primary.starts_with("text/")
            || primary == "application/pdf"
            || primary.contains("document")
            || primary.contains("sheet")
            || primary.contains("presentation")
            || primary.contains("zip")
        {
            Self::Document
        } else {
            Self::Other
        }
    }
}

/// One reaction aggregate: one emoji with its set of reactors.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Reaction {
    pub emoji: String,
    pub participant_ids: Vec<String>,
    pub count: u64,
}

/// Outbound lifecycle exactly as far as the backend attests. `Accepted`
/// means the backend took the send; everything past that is an event, not
/// an assumption. Mirrors the share/notification accepted-vs-effect rule.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    Accepted,
    Sent,
    Delivered,
    Displayed,
    Failed(SendFailure),
}

/// Why a send failed, where attested. `Transport` is a backend-reported
/// failure with no further detail; platform error text never crosses.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SendFailure {
    TooLarge,
    NoRoute,
    Encryption,
    Rejected,
    TimedOut,
    Disconnected,
    Transport,
}

/// A backend-attested status transition for one message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MessageStatusUpdate {
    pub message_id: MessageId,
    pub status: MessageStatus,
}

/// An opaque human-readable verification prompt from the helper (for
/// example, an emoji to confirm on the phone during pairing). Transient:
/// displayed to the user, never stored, never logged with content.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PairingPrompt {
    pub account_id: MessagingAccountId,
    pub prompt: String,
}

/// Read state for the local user in one conversation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReadState {
    pub conversation_id: ConversationId,
    pub last_read_message_id: Option<MessageId>,
    pub unread: bool,
}

/// Currently typing peers in one conversation, backend-attested.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TypingState {
    pub conversation_id: ConversationId,
    pub participant_ids: Vec<String>,
}

/// One explicitly attested backend ability. Membership in a conversation's
/// set means the backend reported it for that conversation; absence means
/// "not attested", never "assumed absent".
///
/// Deliberately absent: message edits, group membership add/remove/rename,
/// disappearing messages. No backend attests first-class operations for
/// those today, so they must not be representable as capabilities.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessagingCapability {
    Text,
    Media,
    Reactions,
    Replies,
    ReadReceipts,
    /// Outbound typing-start only. Upstream cannot send typing-stop, so no
    /// stop semantic is representable here.
    TypingSend,
    TypingReceive,
    GroupCreate,
    ConversationDelete,
    /// Own-device message deletion only.
    MessageDeleteOwn,
    AttachmentDownload,
}

/// A backend-independent messaging operation. Adapters validate against
/// current conversation state and attested capabilities before accepting.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum MessagingCommand {
    SendText {
        conversation_id: ConversationId,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to: Option<MessageId>,
    },
    SendMedia {
        conversation_id: ConversationId,
        /// Local `file://` URL selected by the user.
        file_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
    },
    React {
        message_id: MessageId,
        emoji: String,
    },
    Unreact {
        message_id: MessageId,
        emoji: String,
    },
    MarkRead {
        conversation_id: ConversationId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<MessageId>,
    },
    TypingStart {
        conversation_id: ConversationId,
    },
    DeleteMessage {
        message_id: MessageId,
    },
    OpenConversation {
        account_id: MessagingAccountId,
        addresses: Vec<String>,
    },
}

/// A normalized messaging state transition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum MessagingEvent {
    Account(MessagingAccountEvent),
    Conversation(ConversationEvent),
    Message(MessageEvent),
    Status(MessageStatusUpdate),
    Typing(TypingState),
    Read(ReadState),
    /// Transient verification prompt. Never enters snapshots.
    Pairing(PairingPrompt),
}

impl MessagingCommand {
    pub fn conversation_id(&self) -> ConversationId {
        match self {
            Self::SendText {
                conversation_id, ..
            }
            | Self::SendMedia {
                conversation_id, ..
            }
            | Self::MarkRead {
                conversation_id, ..
            }
            | Self::TypingStart {
                conversation_id, ..
            } => conversation_id.clone(),
            Self::React { message_id, .. }
            | Self::Unreact { message_id, .. }
            | Self::DeleteMessage { message_id } => message_id.conversation_id.clone(),
            Self::OpenConversation { account_id, .. } => ConversationId::new(
                account_id.clone(),
                // Placeholder: opening resolves to a real conversation through
                // the backend; validation only needs the account scope.
                String::new(),
            ),
        }
    }
}

/// Normalize and validate one account record from a helper payload.
pub fn validate_account(account: &MessagingAccount) -> Result<(), ValidationError> {
    check_id(account.id.as_str(), "account id")?;
    check_label(&account.label, "account label")?;
    Ok(())
}

/// Normalize and validate one conversation record from a helper payload.
pub fn validate_conversation(conversation: &Conversation) -> Result<(), ValidationError> {
    check_id(&conversation.id.local_id, "conversation id")?;
    check_id(conversation.id.account_id.as_str(), "account id")?;
    if let Some(title) = &conversation.title {
        check_title(title)?;
    }
    if conversation.participants.is_empty() {
        return Err(ValidationError::EmptyParticipants);
    }
    if conversation.participants.len() > MAX_PARTICIPANTS {
        return Err(ValidationError::TooManyParticipants(
            conversation.participants.len(),
        ));
    }
    let mut seen = BTreeSet::new();
    for participant in &conversation.participants {
        check_id(&participant.local_id, "participant id")?;
        if let Some(name) = &participant.display_name {
            check_title(name)?;
        }
        if let Some(address) = &participant.address {
            check_title(address)?;
        }
        if !seen.insert(participant.local_id.clone()) {
            return Err(ValidationError::DuplicateParticipant(
                participant.local_id.clone(),
            ));
        }
    }
    if let ConversationKind::Direct = conversation.kind {
        if conversation.participants.len() > 2 {
            return Err(ValidationError::DirectWithManyParticipants);
        }
    }
    if let Some(cursor) = &conversation.cursor {
        check_id(cursor, "cursor")?;
    }
    Ok(())
}

/// Normalize and validate one message record from a helper payload.
pub fn validate_message(message: &Message) -> Result<(), ValidationError> {
    check_id(&message.id.local_id, "message id")?;
    check_id(&message.id.conversation_id.local_id, "conversation id")?;
    check_id(message.id.conversation_id.account_id.as_str(), "account id")?;
    check_id(&message.sender.local_id, "sender id")?;
    if let Some(text) = &message.text {
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(ValidationError::TextTooLong(text.chars().count()));
        }
        if text.chars().any(char::is_control) && contains_disallowed_control(text) {
            return Err(ValidationError::InvalidText);
        }
    }
    if message.attachments.len() > MAX_ATTACHMENTS_PER_MESSAGE {
        return Err(ValidationError::TooManyAttachments(
            message.attachments.len(),
        ));
    }
    for attachment in &message.attachments {
        check_id(&attachment.local_id, "attachment id")?;
        if let Some(name) = &attachment.name {
            check_title(name)?;
        }
    }
    if let Some(reply_to) = &message.reply_to {
        if reply_to.conversation_id != message.id.conversation_id {
            return Err(ValidationError::CrossConversationReply);
        }
        check_id(&reply_to.local_id, "reply message id")?;
    }
    if message.reactions.len() > MAX_REACTIONS_PER_MESSAGE {
        return Err(ValidationError::TooManyReactions(message.reactions.len()));
    }
    for reaction in &message.reactions {
        if reaction.emoji.chars().count() > MAX_EMOJI_CHARS || reaction.emoji.is_empty() {
            return Err(ValidationError::InvalidReaction);
        }
        if reaction.participant_ids.len() > MAX_REACTORS_PER_REACTION {
            return Err(ValidationError::TooManyReactors(
                reaction.participant_ids.len(),
            ));
        }
        if reaction.count == 0 || reaction.count < reaction.participant_ids.len() as u64 {
            return Err(ValidationError::InvalidReaction);
        }
    }
    if message.text.is_none() && message.attachments.is_empty() && !message.deleted {
        return Err(ValidationError::EmptyMessage);
    }
    Ok(())
}

/// Validate an outbound command against current normalized state. This
/// checks shape and attested capabilities only; delivery outcome always
/// arrives later as a [`MessageStatusUpdate`].
pub fn validate_command(
    command: &MessagingCommand,
    conversation: Option<&Conversation>,
    message: Option<&Message>,
) -> Result<(), ValidationError> {
    match command {
        MessagingCommand::SendText {
            conversation_id,
            text,
            reply_to,
        } => {
            let conversation = conversation.ok_or(ValidationError::UnknownConversation)?;
            require_conversation_match(&conversation.id, conversation_id)?;
            require_capability(conversation, MessagingCapability::Text)?;
            if let Some(reply_to) = reply_to {
                let target = message.ok_or(ValidationError::UnknownMessage)?;
                if target.id != *reply_to || target.deleted {
                    return Err(ValidationError::UnknownMessage);
                }
                require_capability(conversation, MessagingCapability::Replies)?;
            }
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Err(ValidationError::EmptyText);
            }
            if text.chars().count() > MAX_TEXT_CHARS {
                return Err(ValidationError::TextTooLong(text.chars().count()));
            }
            Ok(())
        }
        MessagingCommand::SendMedia {
            conversation_id,
            caption,
            ..
        } => {
            let conversation = conversation.ok_or(ValidationError::UnknownConversation)?;
            require_conversation_match(&conversation.id, conversation_id)?;
            require_capability(conversation, MessagingCapability::Media)?;
            if let Some(caption) = caption {
                if caption.chars().count() > MAX_TEXT_CHARS {
                    return Err(ValidationError::TextTooLong(caption.chars().count()));
                }
            }
            Ok(())
        }
        MessagingCommand::React { message_id, emoji }
        | MessagingCommand::Unreact { message_id, emoji } => {
            let target = message.ok_or(ValidationError::UnknownMessage)?;
            if target.id != *message_id {
                return Err(ValidationError::UnknownMessage);
            }
            if target.deleted {
                return Err(ValidationError::MessageDeleted);
            }
            let conversation = conversation.ok_or(ValidationError::UnknownConversation)?;
            require_capability(conversation, MessagingCapability::Reactions)?;
            if emoji.is_empty() || emoji.chars().count() > MAX_EMOJI_CHARS {
                return Err(ValidationError::InvalidReaction);
            }
            Ok(())
        }
        MessagingCommand::MarkRead {
            conversation_id, ..
        } => {
            let conversation = conversation.ok_or(ValidationError::UnknownConversation)?;
            require_conversation_match(&conversation.id, conversation_id)?;
            require_capability(conversation, MessagingCapability::ReadReceipts)?;
            Ok(())
        }
        MessagingCommand::TypingStart { conversation_id } => {
            let conversation = conversation.ok_or(ValidationError::UnknownConversation)?;
            require_conversation_match(&conversation.id, conversation_id)?;
            require_capability(conversation, MessagingCapability::TypingSend)?;
            Ok(())
        }
        MessagingCommand::DeleteMessage { message_id } => {
            let target = message.ok_or(ValidationError::UnknownMessage)?;
            if target.id != *message_id {
                return Err(ValidationError::UnknownMessage);
            }
            let conversation = conversation.ok_or(ValidationError::UnknownConversation)?;
            require_capability(conversation, MessagingCapability::MessageDeleteOwn)?;
            if !target.sender.is_self {
                return Err(ValidationError::NotOwnMessage);
            }
            Ok(())
        }
        MessagingCommand::OpenConversation {
            account_id,
            addresses,
        } => {
            if addresses.is_empty() || addresses.len() > MAX_PARTICIPANTS {
                return Err(ValidationError::InvalidAddress);
            }
            for address in addresses {
                let trimmed = address.trim();
                if trimmed.is_empty() || trimmed.chars().count() > MAX_TITLE_CHARS {
                    return Err(ValidationError::InvalidAddress);
                }
            }
            check_id(account_id.as_str(), "account id")?;
            Ok(())
        }
    }
}

fn require_conversation_match(
    current: &ConversationId,
    requested: &ConversationId,
) -> Result<(), ValidationError> {
    if current != requested {
        return Err(ValidationError::UnknownConversation);
    }
    Ok(())
}

fn require_capability(
    conversation: &Conversation,
    capability: MessagingCapability,
) -> Result<(), ValidationError> {
    if conversation.capabilities.contains(&capability) {
        Ok(())
    } else {
        Err(ValidationError::UnsupportedCapability(capability))
    }
}

fn check_id(value: &str, what: &str) -> Result<(), ValidationError> {
    if value.is_empty() || value.len() > MAX_ID_LEN {
        return Err(ValidationError::InvalidId(what.into()));
    }
    if value.chars().any(char::is_control) {
        return Err(ValidationError::InvalidId(what.into()));
    }
    Ok(())
}

fn check_label(value: &str, what: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() || value.chars().count() > MAX_TITLE_CHARS {
        return Err(ValidationError::InvalidLabel(what.into()));
    }
    Ok(())
}

fn check_title(value: &str) -> Result<(), ValidationError> {
    if value.chars().count() > MAX_TITLE_CHARS {
        return Err(ValidationError::InvalidLabel("title".into()));
    }
    Ok(())
}

fn contains_disallowed_control(text: &str) -> bool {
    text.chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
}

/// Sanitize a helper-supplied file name into one safe basename: no path
/// separators, `.`/`..`, NUL, or control characters, at most 255 UTF-8
/// bytes. Returns `None` when nothing safe remains.
pub fn sanitize_file_name(name: &str) -> Option<String> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let trimmed = base.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return None;
    }
    if trimmed.chars().any(|c| c == '\0' || c.is_control()) {
        return None;
    }
    let mut safe = trimmed.to_string();
    while safe.len() > 255 {
        safe.pop();
    }
    if safe.is_empty() || safe == "." || safe == ".." {
        return None;
    }
    Some(safe)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationError {
    InvalidId(String),
    InvalidLabel(String),
    TextTooLong(usize),
    InvalidText,
    EmptyText,
    EmptyMessage,
    EmptyParticipants,
    TooManyParticipants(usize),
    DuplicateParticipant(String),
    DirectWithManyParticipants,
    TooManyAttachments(usize),
    TooManyReactions(usize),
    TooManyReactors(usize),
    InvalidReaction,
    CrossConversationReply,
    UnknownConversation,
    UnknownMessage,
    MessageDeleted,
    NotOwnMessage,
    InvalidAddress,
    UnsupportedCapability(MessagingCapability),
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(what) => write!(formatter, "invalid {what}"),
            Self::InvalidLabel(what) => write!(formatter, "invalid {what}"),
            Self::TextTooLong(len) => {
                write!(
                    formatter,
                    "text too long ({len} chars, max {MAX_TEXT_CHARS})"
                )
            }
            Self::InvalidText => write!(formatter, "text contains invalid characters"),
            Self::EmptyText => write!(formatter, "text must not be empty"),
            Self::EmptyMessage => write!(formatter, "message has no content"),
            Self::EmptyParticipants => write!(formatter, "conversation has no participants"),
            Self::TooManyParticipants(count) => {
                write!(formatter, "too many participants ({count})")
            }
            Self::DuplicateParticipant(id) => {
                write!(formatter, "duplicate participant {id}")
            }
            Self::DirectWithManyParticipants => {
                write!(formatter, "direct conversation has too many participants")
            }
            Self::TooManyAttachments(count) => {
                write!(formatter, "too many attachments ({count})")
            }
            Self::TooManyReactions(count) => write!(formatter, "too many reactions ({count})"),
            Self::TooManyReactors(count) => write!(formatter, "too many reactors ({count})"),
            Self::InvalidReaction => write!(formatter, "invalid reaction"),
            Self::CrossConversationReply => {
                write!(formatter, "reply targets another conversation")
            }
            Self::UnknownConversation => write!(formatter, "conversation is not known"),
            Self::UnknownMessage => write!(formatter, "message is not known"),
            Self::MessageDeleted => write!(formatter, "message was deleted"),
            Self::NotOwnMessage => write!(formatter, "only own messages can be deleted"),
            Self::InvalidAddress => write!(formatter, "invalid conversation address"),
            Self::UnsupportedCapability(capability) => {
                write!(formatter, "conversation does not attest {capability:?}")
            }
        }
    }
}

impl std::error::Error for ValidationError {}
