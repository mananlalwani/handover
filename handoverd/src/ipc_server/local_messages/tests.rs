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
