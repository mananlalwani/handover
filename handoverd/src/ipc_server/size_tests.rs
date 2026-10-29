use super::*;
use handover_core::{ConversationId, MessagingAccountId};
use handover_gmessages::contract::{HelperEvent, WireMessage, WireReaction, decode_event};
use handover_gmessages::normalize::{normalize_account, normalize_message};
use tokio::io::AsyncReadExt;

fn helper_event(message: WireMessage) -> HelperEvent {
    HelperEvent::Messages {
        account: "a".repeat(128),
        conversation: "c".repeat(256),
        messages: vec![message],
        cursor_next: None,
        page_complete: true,
        full: false,
        generation: None,
        fetch_id: Some(1),
    }
}

fn boundary_message() -> handover_core::Message {
    let mut wire = WireMessage {
        local_id: "m".repeat(256),
        sender: "sender".into(),
        transport: None,
        sent_at: None,
        text: Some("body".into()),
        attachments: vec![],
        reply_to: Some("reply".into()),
        deleted: false,
        reactions: (0..32)
            .map(|reaction| WireReaction {
                emoji: format!("r{reaction}"),
                participant_ids: (0..256)
                    .map(|participant| format!("{participant:03}{}", "x".repeat(121)))
                    .collect(),
            })
            .collect(),
    };
    let limit = handover_gmessages::MAX_HELPER_LINE_BYTES;
    let mut extra = limit
        - 16
        - serde_json::to_vec(&helper_event(wire.clone()))
            .unwrap()
            .len();
    for reaction in &mut wire.reactions {
        for participant in &mut reaction.participant_ids {
            let add = extra.min(256 - participant.len());
            participant.push_str(&"y".repeat(add));
            extra -= add;
        }
    }
    assert_eq!(extra, 0);
    let encoded = serde_json::to_vec(&helper_event(wire)).unwrap();
    assert_eq!(encoded.len(), limit - 16);
    let HelperEvent::Messages {
        account,
        conversation,
        mut messages,
        ..
    } = decode_event(&encoded).unwrap()
    else {
        unreachable!()
    };
    normalize_account(&account, "Fixture", true, true).unwrap();
    let id = ConversationId::new(MessagingAccountId::new(account), conversation);
    normalize_message(&id, messages.remove(0)).unwrap()
}

#[tokio::test]
async fn helper_boundary_record_remains_readable_after_normalization() {
    let message = boundary_message();
    let response = ServerMessage::new(ServerPayload::History {
        conversation_id: message.id.conversation_id.clone(),
        messages: vec![message.clone()],
        cursor_next: None,
    });
    let encoded = serde_json::to_vec(&response).unwrap();
    assert!(
        encoded.len() > handover_gmessages::MAX_HELPER_LINE_BYTES,
        "normalization must exercise the transport boundary"
    );
    let (mut writer, mut reader) = tokio::io::duplex(encoded.len() + 1024);
    write_json_line(&mut writer, &response)
        .await
        .expect("an admitted helper message must fit desktop IPC");
    drop(writer);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await.unwrap();
    let decoded: ServerMessage = read_json_line(&mut BufReader::new(bytes.as_slice()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decoded, response);
}

#[tokio::test]
async fn ordinary_snapshot_batches_keep_the_existing_one_mib_target() {
    use handover_core::{DeviceId, Notification, NotificationId};
    let notifications: Vec<_> = (0..400)
        .map(|index| Notification {
            id: NotificationId::new(DeviceId::new("fixture"), format!("notification-{index}")),
            app_name: "Fixture".into(),
            title: "Title".into(),
            body: "x".repeat(4096),
            icon_path: None,
            clearable: true,
            actions: vec![],
            reply_supported: false,
        })
        .collect();
    let payload = ServerPayload::Snapshot {
        devices: vec![],
        notifications: notifications.clone(),
        media_sessions: vec![],
        calls: vec![],
        messaging_accounts: vec![],
        conversations: vec![],
        typing_states: vec![],
        read_states: vec![],
    };
    let size = serde_json::to_vec(&ServerMessage::new(payload.clone()))
        .unwrap()
        .len();
    assert!(size > handover_ipc::PREFERRED_LINE_BYTES && size < handover_ipc::MAX_LINE_BYTES);
    let (mut writer, mut reader) = tokio::io::duplex(2 * handover_ipc::MAX_LINE_BYTES);
    write_snapshot_response(&mut writer, payload).await.unwrap();
    drop(writer);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await.unwrap();
    let lines: Vec<_> = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    assert!(
        lines.len() > 1,
        "the higher individual-record bound must not enlarge ordinary batches"
    );
    let mut actual = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        assert!(line.len() < handover_ipc::PREFERRED_LINE_BYTES);
        let ServerPayload::SnapshotChunk {
            notifications,
            done,
            ..
        } = serde_json::from_slice::<ServerMessage>(line)
            .unwrap()
            .payload
        else {
            panic!("expected snapshot chunk")
        };
        assert_eq!(done, index + 1 == lines.len());
        actual.extend(notifications);
    }
    assert_eq!(actual, notifications);
}
