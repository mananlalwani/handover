use super::*;
use serde::Serialize;
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

pub(crate) async fn write_contacts_response<W>(
    writer: &mut W,
    contacts: Vec<handover_core::Contact>,
) -> Result<(), IpcError>
where
    W: AsyncWrite + Unpin,
{
    if contacts_line_len(&contacts, None)? < handover_ipc::MAX_LINE_BYTES {
        return write_json_line(
            writer,
            &ServerMessage::new(ServerPayload::Contacts { contacts }),
        )
        .await;
    }

    let mut chunk = Vec::new();
    for contact in contacts {
        chunk.push(contact);
        if contacts_line_len(&chunk, Some(false))? >= handover_ipc::MAX_LINE_BYTES {
            let last = chunk.pop().ok_or(IpcError::LineTooLong)?;
            if chunk.is_empty() {
                return Err(IpcError::LineTooLong);
            }
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::ContactsChunk {
                    contacts: chunk,
                    done: false,
                }),
            )
            .await?;
            chunk = vec![last];
            if contacts_line_len(&chunk, Some(false))? >= handover_ipc::MAX_LINE_BYTES {
                return Err(IpcError::LineTooLong);
            }
        }
    }
    write_json_line(
        writer,
        &ServerMessage::new(ServerPayload::ContactsChunk {
            contacts: chunk,
            done: true,
        }),
    )
    .await
}

fn contacts_line_len(
    contacts: &[handover_core::Contact],
    done: Option<bool>,
) -> Result<usize, IpcError> {
    #[derive(Serialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum ContactsResponse<'a> {
        Contacts {
            contacts: &'a [handover_core::Contact],
        },
        ContactsChunk {
            contacts: &'a [handover_core::Contact],
            done: bool,
        },
    }

    #[derive(Serialize)]
    struct Envelope<'a> {
        protocol: u32,
        #[serde(flatten)]
        payload: ContactsResponse<'a>,
    }

    let payload = match done {
        Some(done) => ContactsResponse::ContactsChunk { contacts, done },
        None => ContactsResponse::Contacts { contacts },
    };
    Ok(serde_json::to_vec(&Envelope {
        protocol: handover_ipc::PROTOCOL_VERSION,
        payload,
    })?
    .len())
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
        StateEvent::Contacts(event) => ServerMessage::new(match event {
            handover_core::ContactsEvent::Changed { device_id, count } => {
                ServerPayload::ContactsSynced { device_id, count }
            }
            handover_core::ContactsEvent::Synced {
                device_id,
                contacts,
            } => ServerPayload::ContactsSynced {
                device_id,
                count: contacts.len(),
            },
            handover_core::ContactsEvent::Removed(device_id) => {
                ServerPayload::ContactsRemoved { device_id }
            }
            handover_core::ContactsEvent::SyncFailed { device_id, failure } => {
                ServerPayload::ContactsSyncFailed { device_id, failure }
            }
        }),
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

#[cfg(test)]
mod contacts_tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[test]
    fn contact_events_broadcast_only_sync_metadata() {
        let device_id = DeviceId::new("native:phone-a");
        let synced =
            message_from_event(StateEvent::Contacts(handover_core::ContactsEvent::Synced {
                device_id: device_id.clone(),
                contacts: vec![handover_core::Contact {
                    device_id: device_id.clone(),
                    local_id: "contact-1".into(),
                    display_name: "Private Name".into(),
                    phones: vec!["+15551234567".into()],
                    emails: vec!["private@example.com".into()],
                    photo: Some("private-photo".into()),
                }],
            }));
        assert_eq!(
            synced.payload,
            ServerPayload::ContactsSynced {
                device_id: device_id.clone(),
                count: 1,
            }
        );
        let encoded = serde_json::to_string(&synced).expect("serializes");
        assert!(!encoded.contains("Private Name"));
        assert!(!encoded.contains("private@example.com"));
        assert!(!encoded.contains("private-photo"));
        let changed = message_from_event(StateEvent::Contacts(
            handover_core::ContactsEvent::Changed {
                device_id: device_id.clone(),
                count: 1,
            },
        ));
        assert_eq!(changed, synced);

        let removed = message_from_event(StateEvent::Contacts(
            handover_core::ContactsEvent::Removed(device_id.clone()),
        ));
        assert_eq!(
            removed.payload,
            ServerPayload::ContactsRemoved {
                device_id: device_id.clone()
            }
        );
        let failed = message_from_event(StateEvent::Contacts(
            handover_core::ContactsEvent::SyncFailed {
                device_id: device_id.clone(),
                failure: handover_core::ContactsSyncFailure::Interrupted,
            },
        ));
        assert_eq!(
            failed.payload,
            ServerPayload::ContactsSyncFailed {
                device_id,
                failure: handover_core::ContactsSyncFailure::Interrupted,
            }
        );
    }

    #[tokio::test]
    async fn large_contacts_snapshot_uses_bounded_chunks() {
        let contacts: Vec<_> = (0..100)
            .map(|index| handover_core::Contact {
                device_id: DeviceId::new("native:phone-a"),
                local_id: format!("contact-{index}"),
                display_name: format!("Contact {index}"),
                phones: vec![format!("+1555{index:07}")],
                emails: Vec::new(),
                photo: Some("x".repeat(16_000)),
            })
            .collect();
        let (mut writer, mut reader) = tokio::io::duplex(64 * 1024);
        let write = tokio::spawn(async move {
            write_contacts_response(&mut writer, contacts)
                .await
                .expect("contacts response writes");
        });
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .expect("reads response");
        write.await.expect("writer task completes");

        let lines: Vec<_> = bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .collect();
        assert!(lines.len() > 1);
        let mut all = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            assert!(line.len() < handover_ipc::MAX_LINE_BYTES);
            let message: ServerMessage = serde_json::from_slice(line).expect("valid server line");
            let ServerPayload::ContactsChunk { contacts, done } = message.payload else {
                panic!("large response uses contacts chunks");
            };
            assert_eq!(done, index + 1 == lines.len());
            all.extend(contacts);
        }
        assert_eq!(all.len(), 100);
    }
}
