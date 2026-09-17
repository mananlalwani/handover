//! Tests for backend-independent messaging normalization.

use std::collections::BTreeSet;

use super::messaging::*;

fn sample_account_id() -> MessagingAccountId {
    MessagingAccountId::new("gmessages:default")
}

fn sample_conversation_id() -> ConversationId {
    ConversationId::new(sample_account_id(), "thread-1")
}

fn sample_participant(local_id: &str, is_self: bool) -> Participant {
    Participant {
        local_id: local_id.into(),
        display_name: Some(format!("Name {local_id}")),
        address: Some(format!("+1555000{local_id}")),
        is_self,
    }
}

fn sample_conversation() -> Conversation {
    Conversation {
        id: sample_conversation_id(),
        kind: ConversationKind::Direct,
        transport: TransportKind::Rcs,
        title: None,
        participants: vec![
            sample_participant("self", true),
            sample_participant("peer", false),
        ],
        latest_message_id: None,
        unread_count: Some(2),
        cursor: Some("cursor-9".into()),
        capabilities: BTreeSet::from([
            MessagingCapability::Text,
            MessagingCapability::Media,
            MessagingCapability::Reactions,
            MessagingCapability::Replies,
            MessagingCapability::ReadReceipts,
            MessagingCapability::TypingSend,
            MessagingCapability::TypingReceive,
            MessagingCapability::MessageDeleteOwn,
            MessagingCapability::AttachmentDownload,
        ]),
    }
}

fn sample_message() -> Message {
    Message {
        id: MessageId::new(sample_conversation_id(), "msg-1"),
        sender: sample_participant("peer", false),
        sent_at: Some(1_758_000_000_000_000),
        text: Some("Hello".into()),
        attachments: vec![],
        reply_to: None,
        reactions: vec![],
        deleted: false,
    }
}

#[test]
fn conversation_identity_is_scoped_to_its_account() {
    let first = ConversationId::new(MessagingAccountId::new("gmessages:a"), "t1");
    let second = ConversationId::new(MessagingAccountId::new("gmessages:b"), "t1");

    assert_ne!(first, second);
    assert_eq!(first.to_string(), "gmessages:a:t1");
}

#[test]
fn message_identity_nests_conversation_identity() {
    let id = MessageId::new(sample_conversation_id(), "m1");

    assert_eq!(id.to_string(), "gmessages:default:thread-1:m1");
}

#[test]
fn valid_records_pass_normalization() {
    assert!(
        validate_account(&MessagingAccount {
            id: sample_account_id(),
            label: "Personal".into(),
            connected: true,
            authenticated: true,
        })
        .is_ok()
    );
    assert!(validate_conversation(&sample_conversation()).is_ok());
    assert!(validate_message(&sample_message()).is_ok());
}

#[test]
fn identifiers_reject_empty_and_control_values() {
    let mut conversation = sample_conversation();
    conversation.id.local_id = String::new();
    assert!(validate_conversation(&conversation).is_err());

    let mut message = sample_message();
    message.sender.local_id = "bad\nid".into();
    assert!(validate_message(&message).is_err());
}

#[test]
fn text_bounds_are_enforced() {
    let mut message = sample_message();
    message.text = Some("x".repeat(MAX_TEXT_CHARS + 1));
    assert_eq!(
        validate_message(&message),
        Err(ValidationError::TextTooLong(MAX_TEXT_CHARS + 1))
    );

    assert_eq!(
        validate_command(
            &MessagingCommand::SendText {
                conversation_id: sample_conversation_id(),
                text: "   ".into(),
                reply_to: None,
            },
            Some(&sample_conversation()),
            None,
        ),
        Err(ValidationError::EmptyText)
    );
}

#[test]
fn conversations_reject_bad_participant_sets() {
    let mut conversation = sample_conversation();
    conversation.participants = vec![];
    assert_eq!(
        validate_conversation(&conversation),
        Err(ValidationError::EmptyParticipants)
    );

    let mut conversation = sample_conversation();
    conversation.participants = vec![
        sample_participant("a", false),
        sample_participant("a", false),
    ];
    assert!(matches!(
        validate_conversation(&conversation),
        Err(ValidationError::DuplicateParticipant(_))
    ));

    let mut conversation = sample_conversation();
    conversation
        .participants
        .push(sample_participant("third", false));
    assert_eq!(
        validate_conversation(&conversation),
        Err(ValidationError::DirectWithManyParticipants)
    );
}

#[test]
fn messages_reject_empty_and_cross_conversation_replies() {
    let mut message = sample_message();
    message.text = None;
    assert_eq!(
        validate_message(&message),
        Err(ValidationError::EmptyMessage)
    );

    let mut message = sample_message();
    message.reply_to = Some(MessageId::new(
        ConversationId::new(sample_account_id(), "other-thread"),
        "m0",
    ));
    assert_eq!(
        validate_message(&message),
        Err(ValidationError::CrossConversationReply)
    );
}

#[test]
fn reactions_require_consistent_counts() {
    let mut message = sample_message();
    message.reactions = vec![Reaction {
        emoji: "❤".into(),
        participant_ids: vec!["peer".into()],
        count: 0,
    }];
    assert_eq!(
        validate_message(&message),
        Err(ValidationError::InvalidReaction)
    );

    let mut message = sample_message();
    message.reactions = vec![Reaction {
        emoji: "❤".into(),
        participant_ids: vec!["peer".into()],
        count: 3,
    }];
    assert!(validate_message(&message).is_ok());
}

#[test]
fn commands_require_attested_capabilities() {
    let mut conversation = sample_conversation();
    conversation.capabilities.remove(&MessagingCapability::Text);
    assert_eq!(
        validate_command(
            &MessagingCommand::SendText {
                conversation_id: sample_conversation_id(),
                text: "hi".into(),
                reply_to: None,
            },
            Some(&conversation),
            None,
        ),
        Err(ValidationError::UnsupportedCapability(
            MessagingCapability::Text
        ))
    );

    assert_eq!(
        validate_command(
            &MessagingCommand::TypingStart {
                conversation_id: sample_conversation_id(),
            },
            Some(&sample_conversation()),
            None,
        ),
        Ok(())
    );
}

#[test]
fn delete_requires_own_message_and_capability() {
    let mut own = sample_message();
    own.sender = sample_participant("self", true);
    assert!(
        validate_command(
            &MessagingCommand::DeleteMessage {
                message_id: own.id.clone(),
            },
            Some(&sample_conversation()),
            Some(&own),
        )
        .is_ok()
    );

    assert_eq!(
        validate_command(
            &MessagingCommand::DeleteMessage {
                message_id: sample_message().id.clone(),
            },
            Some(&sample_conversation()),
            Some(&sample_message()),
        ),
        Err(ValidationError::NotOwnMessage)
    );
}

#[test]
fn no_capability_exists_for_edits_or_membership_changes() {
    // Compile-time guard: the attested set must stay closed. If a variant
    // for edits, membership, or disappearing messages is ever added, this
    // test names it explicitly so the addition is deliberate.
    let all = [
        MessagingCapability::Text,
        MessagingCapability::Media,
        MessagingCapability::Reactions,
        MessagingCapability::Replies,
        MessagingCapability::ReadReceipts,
        MessagingCapability::TypingSend,
        MessagingCapability::TypingReceive,
        MessagingCapability::GroupCreate,
        MessagingCapability::ConversationDelete,
        MessagingCapability::MessageDeleteOwn,
        MessagingCapability::AttachmentDownload,
    ];
    assert_eq!(all.len(), 11);
    let serialized = serde_json::to_string(&all).expect("capabilities serialize");
    assert!(!serialized.contains("edit"));
    assert!(!serialized.contains("member"));
    assert!(!serialized.contains("disappear"));
}

#[test]
fn attachment_kinds_derive_from_mime_without_guessing_identity() {
    assert_eq!(
        AttachmentKind::of_mime(Some("image/jpeg")),
        AttachmentKind::Image
    );
    assert_eq!(
        AttachmentKind::of_mime(Some("video/mp4; codecs=avc1")),
        AttachmentKind::Video
    );
    assert_eq!(
        AttachmentKind::of_mime(Some("AUDIO/OGG")),
        AttachmentKind::Audio
    );
    assert_eq!(
        AttachmentKind::of_mime(Some("application/pdf")),
        AttachmentKind::Document
    );
    assert_eq!(AttachmentKind::of_mime(None), AttachmentKind::Other);
    assert_eq!(
        AttachmentKind::of_mime(Some("application/x-dice")),
        AttachmentKind::Other
    );
}

#[test]
fn file_names_are_sanitized_to_one_safe_basename() {
    assert_eq!(
        sanitize_file_name("photo ✓.jpg"),
        Some("photo ✓.jpg".into())
    );
    assert_eq!(
        sanitize_file_name("/tmp/../photo.jpg"),
        Some("photo.jpg".into())
    );
    assert_eq!(sanitize_file_name(".."), None);
    assert_eq!(sanitize_file_name("."), None);
    assert_eq!(sanitize_file_name(""), None);
    assert_eq!(sanitize_file_name("a\0b"), None);
    assert_eq!(sanitize_file_name("a\nb"), None);
    let long = "x".repeat(300);
    assert_eq!(sanitize_file_name(&long).map(|name| name.len()), Some(255));
}

#[test]
fn messaging_wire_forms_carry_no_device_or_protocol_names() {
    let event = MessagingEvent::Message(MessageEvent::Added(sample_message()));
    let json = serde_json::to_string(&event).expect("event serializes");
    assert!(!json.contains("kdeconnect"));
    assert!(!json.contains("bugle"));
    assert!(!json.contains("tachyon"));
    assert_eq!(
        serde_json::from_str::<MessagingEvent>(&json).expect("event deserializes"),
        event
    );

    let command = MessagingCommand::SendText {
        conversation_id: sample_conversation_id(),
        text: "hello".into(),
        reply_to: None,
    };
    let json = serde_json::to_string(&command).expect("command serializes");
    assert!(!json.contains("kdeconnect"));
    assert_eq!(
        serde_json::from_str::<MessagingCommand>(&json).expect("command deserializes"),
        command
    );
}

#[test]
fn status_lifecycle_distinguishes_acceptance_from_effect() {
    for status in [
        MessageStatus::Accepted,
        MessageStatus::Sent,
        MessageStatus::Delivered,
        MessageStatus::Displayed,
        MessageStatus::Failed(SendFailure::Transport),
    ] {
        let update = MessageStatusUpdate {
            message_id: MessageId::new(sample_conversation_id(), "m1"),
            status: status.clone(),
        };
        assert_eq!(
            serde_json::from_str::<MessageStatusUpdate>(
                &serde_json::to_string(&update).expect("serializes")
            )
            .expect("deserializes"),
            update
        );
    }
}
