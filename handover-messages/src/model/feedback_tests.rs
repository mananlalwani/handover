use super::*;

fn account(id: &str) -> MessagingAccount {
    MessagingAccount {
        id: MessagingAccountId::new(id),
        label: id.into(),
        connected: true,
        authenticated: true,
    }
}

fn conversation(account: &str, thread: &str, participants: Vec<Participant>) -> Conversation {
    Conversation {
        id: ConversationId::new(MessagingAccountId::new(account), thread),
        kind: ConversationKind::Direct,
        transport: TransportKind::Unknown,
        title: None,
        participants,
        latest_message_id: None,
        last_activity_at: None,
        unread_count: None,
        cursor: None,
        capabilities: BTreeSet::new(),
    }
}

fn participant(id: &str, name: Option<&str>, is_self: bool) -> Participant {
    Participant {
        local_id: id.into(),
        display_name: name.map(str::to_owned),
        address: None,
        is_self,
    }
}

fn message(id: &ConversationId, local_id: &str, sender: Participant, sent_at: i64) -> Message {
    Message {
        id: MessageId::new(id.clone(), local_id),
        sender,
        transport: None,
        sent_at: Some(sent_at),
        text: Some("same text".into()),
        attachments: vec![],
        reply_to: None,
        reactions: vec![],
        deleted: false,
    }
}

#[test]
fn typing_is_scoped_live_deduplicated_and_never_names_an_unknown_peer() {
    let id = ConversationId::new(MessagingAccountId::new("a"), "thread");
    let mut model = Model {
        connected: true,
        accounts: vec![account("a"), account("b")],
        conversations: vec![
            conversation("a", "thread", vec![participant("self", Some("Me"), true)]),
            conversation(
                "b",
                "thread",
                vec![participant("peer", Some("Other account"), false)],
            ),
        ],
        selected: Some(id.clone()),
        ..Default::default()
    };
    model.state(ServerPayload::Typing {
        state: TypingState {
            conversation_id: id.clone(),
            participant_ids: vec!["ghost".into(), "ghost".into(), "self".into()],
        },
    });
    assert_eq!(model.typing_label().as_deref(), Some("Someone is typing…"));
    model.selected = None;
    assert_eq!(model.typing_label(), None);
    assert_eq!(
        model.typing_label_for(&id).as_deref(),
        Some("Someone is typing…")
    );
    model.selected = Some(id.clone());
    model.state(ServerPayload::Typing {
        state: TypingState {
            conversation_id: id.clone(),
            participant_ids: vec![],
        },
    });
    assert_eq!(model.typing_label(), None);

    model.state(ServerPayload::Typing {
        state: TypingState {
            conversation_id: id,
            participant_ids: vec!["peer".into()],
        },
    });
    assert_eq!(model.typing_label().as_deref(), Some("Someone is typing…"));
    model.accounts[0].connected = false;
    assert_eq!(model.typing_label(), None);
}

#[test]
fn delivery_feedback_attaches_only_by_scoped_message_identity() {
    let id = ConversationId::new(MessagingAccountId::new("a"), "thread");
    let message_id = MessageId::new(id.clone(), "actual");
    let wrong_scope = MessageId::new(
        ConversationId::new(MessagingAccountId::new("b"), "thread"),
        "actual",
    );
    let mut model = Model {
        accounts: vec![account("a"), account("b")],
        ..Default::default()
    };
    model.state(ServerPayload::MessageStatus {
        update: MessageStatusUpdate {
            message_id: message_id.clone(),
            status: MessageStatus::Accepted,
        },
    });
    assert_eq!(model.message_status(&message_id), Some("Accepted"));
    assert_ne!(model.message_status(&message_id), Some("Delivered"));

    let operation =
        |op_id: &str, conv: ConversationId, linked: Option<MessageId>| OutgoingOperation {
            id: op_id.into(),
            conversation_id: conv,
            kind: OutgoingOperationKind::Text,
            created_at: 10,
            updated_at: 10,
            outcome: OutgoingOutcome::Provider(MessageStatus::Delivered),
            message_id: linked,
        };
    model.state(ServerPayload::OutgoingOperation {
        operation: operation("linked", id.clone(), Some(message_id.clone())),
    });
    assert_eq!(model.message_status(&message_id), Some("Delivered"));
    let other = MessageId::new(id.clone(), "other");
    model.state(ServerPayload::OutgoingOperation {
        operation: operation("wrong-scope", id.clone(), Some(wrong_scope)),
    });
    model.state(ServerPayload::OutgoingOperation {
        operation: operation("unlinked", id, None),
    });
    assert_eq!(
        model.message_status(&other),
        None,
        "matching text/time cannot substitute for message_id"
    );
    assert_eq!(model.message_status(&message_id), Some("Delivered"));
}

#[test]
fn new_message_feedback_requires_a_newer_incoming_event_during_older_browsing() {
    let id = ConversationId::new(MessagingAccountId::new("a"), "thread");
    let mut model = Model {
        connected: true,
        selected: Some(id.clone()),
        browsing_older: true,
        conversations: vec![conversation("a", "thread", vec![])],
        ..Default::default()
    };
    let latest = message(&id, "latest", participant("peer", None, false), 20);
    model.remember_preview(&latest);
    let incoming = message(&id, "incoming", participant("peer", None, false), 30);
    model.state(ServerPayload::MessageAdded {
        message: incoming.clone(),
    });
    assert!(model.new_messages.contains(&incoming.id));
    model.state(ServerPayload::MessageAdded {
        message: incoming.clone(),
    });
    let updated = message(&id, "updated", participant("peer", None, false), 40);
    model.state(ServerPayload::MessageUpdated {
        message: updated.clone(),
    });
    model.merge(vec![message(
        &id,
        "history",
        participant("peer", None, false),
        50,
    )]);
    model.state(ServerPayload::MessageAdded {
        message: message(
            &ConversationId::new(id.account_id.clone(), "other-thread"),
            "other",
            participant("peer", None, false),
            60,
        ),
    });
    model.state(ServerPayload::MessageAdded {
        message: message(&id, "self", participant("self", None, true), 70),
    });
    model.state(ServerPayload::MessageAdded {
        message: message(&id, "old", participant("peer", None, false), 10),
    });
    assert_eq!(
        model.new_messages.len(),
        1,
        "updates, history, other thread, self and older events add no count"
    );
    for n in 0..301 {
        let newer = message(
            &id,
            &format!("n{n}"),
            participant("peer", None, false),
            100 + n,
        );
        model.state(ServerPayload::MessageAdded { message: newer });
    }
    assert_eq!(model.new_messages.len(), 300);
    assert!(model.new_messages_overflow);
    model.clear_new_messages();
    assert!(model.new_messages.is_empty());
    assert!(!model.new_messages_overflow);
}

#[test]
fn snapshots_and_removals_clear_transient_feedback() {
    let id = ConversationId::new(MessagingAccountId::new("a"), "thread");
    let message_id = MessageId::new(id.clone(), "m");
    let mut model = Model {
        accounts: vec![account("a")],
        conversations: vec![conversation("a", "thread", vec![])],
        ..Default::default()
    };
    model
        .statuses
        .insert(message_id.clone(), MessageStatus::Delivered);
    model.new_messages.insert(message_id.clone());
    model.outgoing.insert(
        "op".into(),
        OutgoingOperation {
            id: "op".into(),
            conversation_id: id.clone(),
            kind: OutgoingOperationKind::Text,
            created_at: 1,
            updated_at: 1,
            outcome: OutgoingOutcome::Unknown,
            message_id: Some(message_id.clone()),
        },
    );
    model.state(ServerPayload::Typing {
        state: TypingState {
            conversation_id: id.clone(),
            participant_ids: vec!["peer".into()],
        },
    });
    model.state(ServerPayload::Snapshot {
        devices: vec![],
        notifications: vec![],
        media_sessions: vec![],
        calls: vec![],
        messaging_accounts: vec![account("a")],
        conversations: vec![conversation("a", "thread", vec![])],
        typing_states: vec![],
        read_states: vec![],
    });
    assert!(
        model.statuses.is_empty() && model.outgoing.is_empty() && model.new_messages.is_empty()
    );
    assert!(model.typing.is_empty());

    model
        .statuses
        .insert(message_id.clone(), MessageStatus::Sent);
    model.new_messages.insert(message_id.clone());
    model.state(ServerPayload::ConversationRemoved {
        conversation_id: id.clone(),
    });
    assert!(model.statuses.is_empty() && model.new_messages.is_empty());
}

#[test]
fn unread_dots_follow_reported_state_and_preserve_unknown() {
    let a = ConversationId::new(MessagingAccountId::new("a"), "thread");
    let b = ConversationId::new(MessagingAccountId::new("b"), "thread");
    let mut first = conversation("a", "thread", vec![]);
    first.unread_count = Some(3);
    let mut second = conversation("b", "thread", vec![]);
    second.unread_count = Some(2);
    let mut model = Model {
        conversations: vec![first.clone(), second],
        ..Default::default()
    };
    model.reset_read(vec![ReadState {
        conversation_id: a.clone(),
        last_read_message_id: None,
        unread: false,
    }]);
    assert_eq!(model.unread(&a), Some(false));
    assert_eq!(model.unread(&b), Some(true));
    first.unread_count = None;
    model.state(ServerPayload::ConversationUpdated {
        conversation: first.clone(),
    });
    assert_eq!(
        model.unread(&a),
        Some(false),
        "unknown inventory counts do not erase read evidence"
    );
    first.unread_count = Some(1);
    model.state(ServerPayload::ConversationUpdated {
        conversation: first.clone(),
    });
    assert_eq!(
        model.unread(&a),
        Some(true),
        "a later explicit count supersedes old read evidence"
    );
    model.state(ServerPayload::ReadState {
        state: ReadState {
            conversation_id: a.clone(),
            last_read_message_id: None,
            unread: false,
        },
    });
    assert_eq!(model.unread(&a), Some(false));
    model.state(ServerPayload::ConversationRemoved {
        conversation_id: a.clone(),
    });
    first.unread_count = None;
    model.state(ServerPayload::ConversationAdded {
        conversation: first,
    });
    assert_eq!(
        model.unread(&a),
        None,
        "removed read evidence must not leak into a new conversation"
    );
    model.reset_read(vec![]);
    assert_eq!(model.unread(&a), None);
}

#[test]
fn reaction_ownership_requires_a_known_self_actor_in_the_same_conversation() {
    let a = ConversationId::new(MessagingAccountId::new("a"), "thread");
    let b = ConversationId::new(MessagingAccountId::new("b"), "thread");
    let mut incoming = message(&a, "message", participant("peer", None, false), 1);
    incoming.reactions = vec![
        Reaction {
            emoji: "👍".into(),
            participant_ids: vec!["peer".into()],
            count: 1,
        },
        Reaction {
            emoji: "❤️".into(),
            participant_ids: vec!["peer".into(), "self-a".into()],
            count: 2,
        },
        Reaction {
            emoji: "😂".into(),
            participant_ids: vec!["unknown".into()],
            count: 1,
        },
        Reaction {
            emoji: "😮".into(),
            participant_ids: vec![],
            count: 1,
        },
    ];
    let mut foreign = incoming.clone();
    foreign.id.conversation_id = b.clone();
    let mut model = Model {
        conversations: vec![
            conversation(
                "a",
                "thread",
                vec![
                    participant("self-a", None, true),
                    participant("peer", None, false),
                ],
            ),
            conversation("b", "thread", vec![participant("self-b", None, true)]),
        ],
        messages: vec![incoming.clone(), foreign.clone()],
        ..Default::default()
    };
    assert!(!model.has_own_reaction(&incoming.id, "👍"));
    assert!(model.has_own_reaction(&incoming.id, "❤️"));
    assert!(!model.has_own_reaction(&incoming.id, "😂"));
    assert!(!model.has_own_reaction(&incoming.id, "😮"));
    assert!(!model.has_own_reaction(&foreign.id, "❤️"));
    model.messages[0].deleted = true;
    assert!(!model.has_own_reaction(&incoming.id, "❤️"));
    model.messages.clear();
    assert!(!model.has_own_reaction(&incoming.id, "❤️"));
}
