use super::*;
use std::collections::VecDeque;

pub(crate) async fn write_outgoing_operations<W>(
    writer: &mut W,
    operations: Vec<handover_core::OutgoingOperation>,
) -> Result<(), IpcError>
where
    W: AsyncWrite + Unpin,
{
    let mut chunks = Vec::new();
    let mut remaining: VecDeque<_> = operations.into();
    loop {
        let mut chunk = Vec::new();
        for _ in 0..MAX_OUTGOING_CHUNK_ITEMS {
            let Some(operation) = remaining.pop_front() else {
                break;
            };
            chunk.push(operation);
        }
        if chunk.is_empty() {
            if chunks.is_empty() {
                chunks.push(Vec::new());
            }
            break;
        }
        loop {
            let probe = ServerMessage::new(ServerPayload::OutgoingOperations {
                operations: chunk.clone(),
                done: false,
            });
            if serde_json::to_vec(&probe)?.len() < handover_ipc::MAX_LINE_BYTES {
                break;
            }
            let Some(last) = chunk.pop() else {
                return Err(IpcError::LineTooLong);
            };
            if chunk.is_empty() {
                return Err(IpcError::LineTooLong);
            }
            remaining.push_front(last);
        }
        chunks.push(chunk);
        if remaining.is_empty() {
            break;
        }
    }
    let count = chunks.len();
    for (index, operations) in chunks.into_iter().enumerate() {
        write_json_line(
            writer,
            &ServerMessage::new(ServerPayload::OutgoingOperations {
                operations,
                done: index + 1 == count,
            }),
        )
        .await?;
    }
    Ok(())
}

pub(crate) fn snapshot(state: &Arc<RwLock<StateStore>>) -> StateSnapshot {
    state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .snapshot()
}

pub(crate) struct MessagingSnapshot {
    pub(crate) accounts: Vec<handover_core::MessagingAccount>,
    pub(crate) conversations: Vec<handover_core::Conversation>,
    pub(crate) typing: Vec<handover_core::TypingState>,
    pub(crate) read: Vec<handover_core::ReadState>,
}

pub(crate) fn messaging_snapshot(state: &Arc<RwLock<StateStore>>) -> MessagingSnapshot {
    let guard = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    MessagingSnapshot {
        accounts: guard.messaging().snapshot_accounts(),
        conversations: guard.messaging().snapshot_conversations(),
        typing: guard.messaging().snapshot_typing(),
        read: guard.messaging().snapshot_read(),
    }
}

pub(crate) fn snapshot_payload(
    state: &Arc<RwLock<StateStore>>,
    include_media: bool,
    include_messages: bool,
) -> ServerPayload {
    let snapshot = snapshot(state);
    let messaging = messaging_snapshot(state);
    ServerPayload::Snapshot {
        devices: snapshot.devices,
        calls: snapshot.calls,
        notifications: snapshot.notifications,
        media_sessions: if include_media {
            snapshot.media_sessions
        } else {
            Vec::new()
        },
        messaging_accounts: if include_messages {
            messaging.accounts
        } else {
            Vec::new()
        },
        conversations: if include_messages {
            messaging.conversations
        } else {
            Vec::new()
        },
        typing_states: if include_messages {
            messaging.typing
        } else {
            Vec::new()
        },
        read_states: if include_messages {
            messaging.read
        } else {
            Vec::new()
        },
    }
}

pub(crate) fn message_from_event(event: StateEvent) -> ServerMessage {
    match event {
        StateEvent::Device(event) => ServerMessage::from_device_event(event),
        StateEvent::Notification(event) => ServerMessage::from_notification_event(event),
        StateEvent::Media(event) => ServerMessage::from_media_event(event),
        StateEvent::Call(event) => ServerMessage::from_call_event(event),
        StateEvent::CallCommandResult(result) => {
            ServerMessage::new(ServerPayload::CallCommandResult { result })
        }
        StateEvent::DeviceCommandResult(result) => {
            ServerMessage::new(ServerPayload::DeviceCommandResult { result })
        }
        StateEvent::Messaging(event) => ServerMessage::from_messaging_event(event),
        StateEvent::ShareReceived(share) => {
            ServerMessage::new(ServerPayload::ShareReceived { share })
        }
        StateEvent::ShareProgress(progress) => {
            ServerMessage::new(ServerPayload::ShareProgress { progress })
        }
        StateEvent::ShareResult(result) => {
            ServerMessage::new(ServerPayload::ShareResult { result })
        }
        StateEvent::Filesystem(result) => ServerMessage::new(ServerPayload::Filesystem { result }),
        StateEvent::Presentation(_) => ServerMessage::protocol_error(
            ErrorCode::BackendRejected,
            "presentation commands are not broadcast to desktop clients",
        ),
        StateEvent::Volume(_) => ServerMessage::protocol_error(
            ErrorCode::BackendRejected,
            "volume commands are not broadcast to desktop clients",
        ),
        StateEvent::Contacts(_) => ServerMessage::protocol_error(
            ErrorCode::BackendRejected,
            "contact snapshots are requested explicitly",
        ),
        StateEvent::Clipboard(_) => ServerMessage::protocol_error(
            ErrorCode::BackendRejected,
            "clipboard changes are not broadcast to desktop clients",
        ),
        StateEvent::ClipboardFile(_) => ServerMessage::protocol_error(
            ErrorCode::BackendRejected,
            "clipboard changes are not broadcast to desktop clients",
        ),
        StateEvent::RemoteInput(_) => ServerMessage::protocol_error(
            ErrorCode::BackendRejected,
            "remote input events are not broadcast to desktop clients",
        ),
    }
}

pub(crate) fn messaging_validation_error(
    error: &MessagingValidationError,
) -> (ErrorCode, &'static str) {
    match error {
        MessagingValidationError::UnknownAccount => (
            ErrorCode::UnknownMessagingAccount,
            "messaging account is not known",
        ),
        MessagingValidationError::UnknownConversation => {
            (ErrorCode::UnknownConversation, "conversation is not known")
        }
        MessagingValidationError::UnknownMessage => {
            (ErrorCode::UnknownMessage, "message is not known")
        }
        MessagingValidationError::AccountUnavailable => (
            ErrorCode::MessagingUnavailable,
            "messaging account is unavailable",
        ),
        MessagingValidationError::Invalid(
            handover_core::ValidationError::UnsupportedCapability(_),
        ) => (
            ErrorCode::UnsupportedMessagingCapability,
            "conversation does not attest that capability",
        ),
        MessagingValidationError::Invalid(_) => (
            ErrorCode::InvalidMessagingCommand,
            "invalid messaging command",
        ),
    }
}

pub(crate) fn helper_call_error(error: HelperCallError) -> (ErrorCode, String) {
    match error {
        HelperCallError::Unavailable | HelperCallError::NotSubmitted => (
            ErrorCode::MessagingUnavailable,
            "messaging helper is unavailable".into(),
        ),
        HelperCallError::Busy => (
            ErrorCode::BackendRejected,
            "messaging helper is busy".into(),
        ),
        HelperCallError::Timeout => (
            ErrorCode::BackendRejected,
            "messaging helper timed out".into(),
        ),
        HelperCallError::Rejected(reason) if !reason.is_empty() => {
            (ErrorCode::BackendRejected, reason)
        }
        HelperCallError::Rejected(_) => (
            ErrorCode::BackendRejected,
            "messaging helper rejected the command".into(),
        ),
        HelperCallError::BadBundle => (
            ErrorCode::CredentialRejected,
            "credential bundle was rejected".into(),
        ),
    }
}

pub(crate) fn require_messaging_hub(
    messaging: &Option<MessagingHub>,
) -> Result<&MessagingHub, (ErrorCode, String)> {
    messaging
        .as_ref()
        .ok_or_else(|| helper_call_error(HelperCallError::Unavailable))
}
