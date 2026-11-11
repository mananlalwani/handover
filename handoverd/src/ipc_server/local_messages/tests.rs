use super::*;
use handover_core::messaging::{MAX_ATTACHMENTS_PER_MESSAGE, MAX_TITLE_CHARS};
use handover_core::{
    Attachment, Conversation, ConversationEvent, ConversationId, ConversationKind, Message,
    MessageDraft, MessageId, MessagingAccount, MessagingAccountEvent, MessagingAccountId,
    MessagingEvent, Participant, StateEvent, TransportKind,
};
use handover_ipc::{ErrorCode, ServerMessage, ServerPayload, read_json_line};
use std::sync::{Arc, RwLock};
use tokio::io::BufReader;

fn identity() -> (MessagingAccountId, ConversationId) {
    let account = MessagingAccountId::new("fixture");
    let conversation = ConversationId::new(account.clone(), "thread-1");
    (account, conversation)
}

fn message(
    conversation_id: &ConversationId,
    id: impl Into<String>,
    sent_at: i64,
    text: &str,
) -> Message {
    Message {
        id: MessageId::new(conversation_id.clone(), id),
        sender: Participant {
            local_id: "peer".into(),
            display_name: None,
            address: None,
            is_self: false,
        },
        transport: None,
        sent_at: Some(sent_at),
        text: Some(text.into()),
        attachments: Vec::new(),
        reply_to: None,
        reactions: Vec::new(),
        deleted: false,
    }
}

fn state_with_account(conversation_id: &ConversationId) -> Arc<RwLock<crate::state::StateStore>> {
    let mut state = crate::state::StateStore::default();
    state.apply(StateEvent::Messaging(MessagingEvent::Account(
        MessagingAccountEvent::Added(MessagingAccount {
            id: conversation_id.account_id.clone(),
            label: "Offline fixture".into(),
            connected: false,
            authenticated: false,
        }),
    )));
    state.apply(StateEvent::Messaging(MessagingEvent::Conversation(
        ConversationEvent::Added(Conversation {
            id: conversation_id.clone(),
            kind: ConversationKind::Direct,
            transport: TransportKind::Unknown,
            title: Some("Peer".into()),
            participants: Vec::new(),
            latest_message_id: None,
            last_activity_at: None,
            unread_count: None,
            cursor: None,
            capabilities: Default::default(),
        }),
    )));
    Arc::new(RwLock::new(state))
}

async fn call(state: &Arc<RwLock<crate::state::StateStore>>, method: Method) -> Vec<ServerMessage> {
    let (mut writer, reader) = tokio::io::duplex(handover_ipc::MAX_LINE_BYTES * 3);
    assert!(handle(method, &mut writer, state).await.unwrap());
    drop(writer);
    let mut reader = BufReader::new(reader);
    let mut messages = Vec::new();
    while let Some(message) = read_json_line::<_, ServerMessage>(&mut reader)
        .await
        .unwrap()
    {
        messages.push(message);
    }
    messages
}

#[tokio::test]
async fn archived_search_and_context_work_while_account_is_offline() {
    let (account, conversation) = identity();
    let dir = tempfile::tempdir().unwrap();
    let archive =
        crate::messaging_archive::MessageArchive::open(&dir.path().join("history.sqlite")).unwrap();
    archive
        .merge(vec![message(
            &conversation,
            "archived",
            12,
            "needle in saved history",
        )])
        .await
        .unwrap();
    let state = state_with_account(&conversation);
    state.write().unwrap().message_archive = Some(archive);

    let search = call(
        &state,
        Method::MessagesSearch {
            query: "needle".into(),
            account_id: Some(account),
            offset: 0,
        },
    )
    .await;
    assert!(
        matches!(&search[0].payload, ServerPayload::MessageSearch { hits, has_more: false }
        if hits.len() == 1 && hits[0].message_id.local_id == "archived")
    );
    let context = call(
        &state,
        Method::MessagesContext {
            message_id: MessageId::new(conversation, "archived"),
        },
    )
    .await;
    assert!(
        matches!(&context[..], [ServerMessage { payload: ServerPayload::MessageContext { messages, done: true, .. }, .. }]
        if messages.len() == 1 && messages[0].text.as_deref() == Some("needle in saved history"))
    );
}

#[tokio::test]
async fn previews_return_newest_saved_message_per_requested_account_and_omit_empty_threads() {
    let account_a = MessagingAccountId::new("fixture-a");
    let account_b = MessagingAccountId::new("fixture-b");
    let conversation_a = ConversationId::new(account_a.clone(), "same-thread");
    let conversation_b = ConversationId::new(account_b.clone(), "same-thread");
    let empty = ConversationId::new(account_a.clone(), "no-history");
    let dir = tempfile::tempdir().unwrap();
    let archive =
        crate::messaging_archive::MessageArchive::open(&dir.path().join("history.sqlite")).unwrap();
    archive
        .merge(vec![
            message(&conversation_a, "older", 1, "older saved"),
            message(&conversation_a, "newest", 2, "newest saved"),
            message(&conversation_b, "other-account", 3, "other account"),
        ])
        .await
        .unwrap();
    let state = state_with_account(&conversation_a);
    state
        .write()
        .unwrap()
        .apply(StateEvent::Messaging(MessagingEvent::Account(
            MessagingAccountEvent::Added(MessagingAccount {
                id: account_b,
                label: "Second offline fixture".into(),
                connected: false,
                authenticated: false,
            }),
        )));
    state.write().unwrap().message_archive = Some(archive);

    let response = call(
        &state,
        Method::MessagesPreviews {
            conversation_ids: vec![conversation_a, empty, conversation_b],
        },
    )
    .await;
    assert!(matches!(
        &response[..],
        [ServerMessage {
            payload: ServerPayload::MessagePreviews { messages, done: true },
            ..
        }] if messages.len() == 2
            && messages[0].id.local_id == "newest"
            && messages[0].text.as_deref() == Some("newest saved")
            && messages[1].id.local_id == "other-account"
    ));
}

#[tokio::test]
async fn previews_reject_unknown_accounts_invalid_ids_and_more_than_fifty_ids() {
    let (_, conversation) = identity();
    let state = state_with_account(&conversation);
    let unknown = ConversationId::new(MessagingAccountId::new("not-configured"), "thread");
    let invalid = ConversationId::new(conversation.account_id.clone(), "bad\nthread");
    let too_many = (0..51)
        .map(|index| {
            ConversationId::new(conversation.account_id.clone(), format!("thread-{index}"))
        })
        .collect();

    for (ids, code) in [
        (vec![unknown], ErrorCode::UnknownMessagingAccount),
        (vec![invalid], ErrorCode::InvalidMessagingCommand),
        (too_many, ErrorCode::InvalidMessagingCommand),
    ] {
        let response = call(
            &state,
            Method::MessagesPreviews {
                conversation_ids: ids,
            },
        )
        .await;
        assert!(matches!(
            &response[..],
            [ServerMessage {
                payload: ServerPayload::Error { code: actual, .. },
                ..
            }] if *actual == code
        ));
    }
}

#[tokio::test]
async fn previews_chunk_large_records_within_frame_bounds_and_preserve_all_records() {
    let account = MessagingAccountId::new("fixture");
    let conversations: Vec<_> = (0..40)
        .map(|index| ConversationId::new(account.clone(), format!("thread-{index}")))
        .collect();
    let state = state_with_account(&conversations[0]);
    let dir = tempfile::tempdir().unwrap();
    let archive =
        crate::messaging_archive::MessageArchive::open(&dir.path().join("history.sqlite")).unwrap();
    let messages: Vec<_> = conversations
        .iter()
        .enumerate()
        .map(|(index, conversation)| {
            message(
                conversation,
                format!("preview-{index}"),
                index as i64,
                &"📱".repeat(handover_core::MAX_TEXT_CHARS),
            )
        })
        .collect();
    for item in &messages {
        handover_core::validate_message(item).unwrap();
    }
    archive.merge(messages.clone()).await.unwrap();
    state.write().unwrap().message_archive = Some(archive);

    let (mut writer, reader) = tokio::io::duplex(handover_ipc::MAX_LINE_BYTES * 64);
    assert!(
        handle(
            Method::MessagesPreviews {
                conversation_ids: conversations,
            },
            &mut writer,
            &state,
        )
        .await
        .unwrap()
    );
    drop(writer);
    let mut reader = BufReader::new(reader);
    let mut frames = Vec::new();
    while let Some(frame) = read_json_line::<_, ServerMessage>(&mut reader)
        .await
        .unwrap()
    {
        frames.push(frame);
    }
    assert!(
        frames.len() > 1,
        "large previews should use multiple IPC frames"
    );
    let mut got = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        assert!(serde_json::to_vec(frame).unwrap().len() < handover_ipc::MAX_LINE_BYTES);
        match &frame.payload {
            ServerPayload::MessagePreviews { messages, done } => {
                assert_eq!(*done, index + 1 == frames.len());
                got.extend(messages.iter().map(|message| message.id.clone()));
            }
            other => panic!("unexpected frame: {other:?}"),
        }
    }
    let mut expected = messages
        .iter()
        .map(|message| message.id.clone())
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(got, expected);
}

#[tokio::test]
async fn unknown_account_filters_reject_search_context_and_draft_access() {
    let (_, conversation) = identity();
    let dir = tempfile::tempdir().unwrap();
    let archive =
        crate::messaging_archive::MessageArchive::open(&dir.path().join("history.sqlite")).unwrap();
    archive
        .merge(vec![message(&conversation, "secret", 1, "private phrase")])
        .await
        .unwrap();
    let state = state_with_account(&conversation);
    state.write().unwrap().message_archive = Some(archive);
    let unknown = MessagingAccountId::new("not-configured");
    let hidden = ConversationId::new(unknown.clone(), conversation.local_id.clone());

    for method in [
        Method::MessagesSearch {
            query: "private".into(),
            account_id: Some(unknown),
            offset: 0,
        },
        Method::MessagesContext {
            message_id: MessageId::new(hidden.clone(), "secret"),
        },
        Method::MessagesDraft {
            conversation_id: hidden,
        },
    ] {
        let messages = call(&state, method).await;
        assert!(matches!(
            &messages[..],
            [ServerMessage {
                payload: ServerPayload::Error {
                    code: ErrorCode::UnknownMessagingAccount,
                    ..
                },
                ..
            }]
        ));
    }
}

#[tokio::test]
async fn workspace_draft_revision_cas_and_clear_are_enforced_over_ipc() {
    let (_, conversation) = identity();
    let state = state_with_account(&conversation);
    let save = |text: &str, revision| Method::MessagesSaveDraft {
        draft: MessageDraft {
            conversation_id: conversation.clone(),
            text: text.into(),
            reply_to: None,
            revision,
        },
    };
    let first = call(&state, save("first", 0)).await;
    let revision = match &first[0].payload {
        ServerPayload::MessageDraft { draft } => draft.revision,
        other => panic!("unexpected response: {other:?}"),
    };
    assert!(revision > 0);
    let stale = call(&state, save("stale overwrite", 0)).await;
    assert!(matches!(
        &stale[..],
        [ServerMessage {
            payload: ServerPayload::Error {
                code: ErrorCode::DraftConflict,
                ..
            },
            ..
        }]
    ));
    let other_conversation = ConversationId::new(conversation.account_id.clone(), "other-thread");
    let invalid_scope = call(
        &state,
        Method::MessagesSaveDraft {
            draft: MessageDraft {
                conversation_id: conversation.clone(),
                text: "reply".into(),
                reply_to: Some(MessageId::new(other_conversation, "foreign-message")),
                revision,
            },
        },
    )
    .await;
    assert!(matches!(
        &invalid_scope[..],
        [ServerMessage {
            payload: ServerPayload::Error {
                code: ErrorCode::InvalidMessagingCommand,
                ..
            },
            ..
        }]
    ));
    let cleared = call(&state, save("", revision)).await;
    assert!(
        matches!(&cleared[0].payload, ServerPayload::MessageDraft { draft } if draft.text.is_empty() && draft.revision == 0)
    );
    let workspace = call(&state, Method::MessagesWorkspace).await;
    assert!(
        matches!(&workspace[0].payload, ServerPayload::MessagingWorkspace { workspace } if !workspace.persistent && workspace.draft_conversations.is_empty())
    );
}

#[tokio::test]
async fn context_chunks_large_valid_records_within_frame_bounds_and_preserves_all_messages() {
    let (_, conversation) = identity();
    let state = state_with_account(&conversation);
    let dir = tempfile::tempdir().unwrap();
    let archive =
        crate::messaging_archive::MessageArchive::open(&dir.path().join("history.sqlite")).unwrap();
    let messages: Vec<_> = (0..22)
        .map(|n| {
            let mut message = message(
                &conversation,
                format!("m{n:02}"),
                n,
                &"📱".repeat(handover_core::MAX_TEXT_CHARS),
            );
            message.attachments = (0..MAX_ATTACHMENTS_PER_MESSAGE)
                .map(|i| Attachment {
                    local_id: format!("attachment-{i}"),
                    mime: None,
                    name: Some("📎".repeat(MAX_TITLE_CHARS)),
                    size_bytes: None,
                    staged_path: None,
                })
                .collect();
            message
        })
        .collect();
    for item in &messages {
        handover_core::validate_message(item).unwrap();
    }
    archive.merge(messages.clone()).await.unwrap();
    state.write().unwrap().message_archive = Some(archive);
    let frames = call(
        &state,
        Method::MessagesContext {
            message_id: messages[10].id.clone(),
        },
    )
    .await;
    assert!(
        frames.len() > 1,
        "large context should use multiple IPC frames"
    );
    let mut got = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        let encoded = serde_json::to_vec(frame).unwrap();
        assert!(encoded.len() < handover_ipc::MAX_LINE_BYTES);
        match &frame.payload {
            ServerPayload::MessageContext { messages, done, .. } => {
                assert_eq!(*done, index + 1 == frames.len());
                got.extend(messages.iter().map(|message| message.id.clone()));
            }
            other => panic!("unexpected frame: {other:?}"),
        }
    }
    assert_eq!(
        got,
        messages
            .iter()
            .map(|message| message.id.clone())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn workspace_reports_persistence_for_file_backed_store() {
    let (_, conversation) = identity();
    let state = state_with_account(&conversation);
    let dir = tempfile::tempdir().unwrap();
    state.write().unwrap().message_workspace =
        crate::messaging_workspace::Workspace::open(&dir.path().join("workspace.sqlite")).unwrap();
    let workspace = call(&state, Method::MessagesWorkspace).await;
    assert!(
        matches!(&workspace[0].payload, ServerPayload::MessagingWorkspace { workspace } if workspace.persistent)
    );
}

#[tokio::test]
async fn saved_context_exposes_an_older_history_continuation() {
    let (_, conversation) = identity();
    let directory = tempfile::tempdir().unwrap();
    let archive =
        crate::messaging_archive::MessageArchive::open(&directory.path().join("history.sqlite"))
            .unwrap();
    archive
        .store_page(
            conversation.clone(),
            None,
            (0..50)
                .map(|n| message(&conversation, format!("m{n:03}"), n, "saved message"))
                .collect(),
            Some("provider-older".into()),
        )
        .await
        .unwrap();
    let state = state_with_account(&conversation);
    state.write().unwrap().message_archive = Some(archive);
    let replies = call(
        &state,
        Method::MessagesContext {
            message_id: MessageId::new(conversation, "m025"),
        },
    )
    .await;
    match &replies[0].payload {
        ServerPayload::MessageContext {
            messages,
            cursor_next,
            done,
            ..
        } => {
            assert!(*done);
            assert_eq!(
                cursor_next.as_deref(),
                Some(messages[0].id.local_id.as_str())
            );
        }
        _ => panic!("expected saved context with continuation"),
    }
}
