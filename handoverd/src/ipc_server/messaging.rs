use super::*;

pub(crate) async fn handle_messaging_send<W>(
    conversation_id: ConversationId,
    text: String,
    reply_to: Option<MessageId>,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let account = conversation_id.account_id.as_str().to_string();
    let conversation = conversation_id.local_id.clone();
    request_messaging(
        MessagingCommand::SendText {
            conversation_id,
            text: text.clone(),
            reply_to: reply_to.clone(),
        },
        move |request_id| handover_gmessages::contract::HelperCommand::SendText {
            request_id,
            account,
            conversation,
            text,
            reply_to: reply_to.map(|id| id.local_id),
        },
        writer,
        state,
        messaging,
    )
    .await
}

pub(crate) async fn handle_send_file<W>(
    conversation_id: ConversationId,
    file_url: String,
    caption: Option<String>,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let url = match validate_file_url(&file_url).await {
        Ok(url) => url,
        Err(code) => {
            write_json_line(writer, &ServerMessage::protocol_error(code, "invalid file")).await?;
            return Ok(true);
        }
    };
    let path = Url::parse(&url)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .and_then(|path| path.to_str().map(str::to_string));
    let Some(path) = path else {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(ErrorCode::InvalidResource, "invalid file"),
        )
        .await?;
        return Ok(true);
    };
    // Bound outbound attachments before the helper ever reads them.
    let oversized = std::fs::metadata(&path)
        .map(|metadata| {
            !metadata.is_file() || metadata.len() > handover_gmessages::staging::MAX_STAGED_BYTES
        })
        .unwrap_or(true);
    if oversized {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(
                ErrorCode::InvalidMessagingCommand,
                "file is not usable",
            ),
        )
        .await?;
        return Ok(true);
    }
    // Copy into daemon-owned staging before the helper opens it: the
    // source may live in a directory writable by another local user.
    // The adapter accepts this root and opens the private copy;
    // retention sweeps it later (see operations docs).
    let staged = match handover_gmessages::staging::stage_send_copy(&path) {
        Ok(staged) => staged,
        Err(_) => {
            write_json_line(
                writer,
                &ServerMessage::protocol_error(
                    ErrorCode::InvalidMessagingCommand,
                    "file is not usable",
                ),
            )
            .await?;
            return Ok(true);
        }
    };
    let path = staged.to_str().map(str::to_string);
    let Some(path) = path else {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(ErrorCode::InvalidMessagingCommand, "invalid file"),
        )
        .await?;
        return Ok(true);
    };
    let account = conversation_id.account_id.as_str().to_string();
    let conversation = conversation_id.local_id.clone();
    request_messaging(
        MessagingCommand::SendMedia {
            conversation_id,
            file_url: url.clone(),
            caption: caption.clone(),
        },
        move |request_id| handover_gmessages::contract::HelperCommand::SendMedia {
            request_id,
            account,
            conversation,
            path,
            caption,
        },
        writer,
        state,
        messaging,
    )
    .await
}

pub(crate) async fn handle_react<W>(
    message_id: MessageId,
    emoji: String,
    add: bool,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let account = message_id.conversation_id.account_id.as_str().to_string();
    let conversation = message_id.conversation_id.local_id.clone();
    let message = message_id.local_id.clone();
    let command = if add {
        MessagingCommand::React {
            message_id: message_id.clone(),
            emoji: emoji.clone(),
        }
    } else {
        MessagingCommand::Unreact {
            message_id: message_id.clone(),
            emoji: emoji.clone(),
        }
    };
    request_messaging(
        command,
        move |request_id| handover_gmessages::contract::HelperCommand::React {
            request_id,
            account,
            conversation,
            message,
            emoji,
            add,
        },
        writer,
        state,
        messaging,
    )
    .await
}

pub(crate) async fn handle_mark_read<W>(
    conversation_id: ConversationId,
    message_id: Option<MessageId>,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    // Default to the newest stored message; the helper attests the effect.
    let resolved = message_id
        .as_ref()
        .map(|id| id.local_id.clone())
        .or_else(|| {
            state
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .messaging()
                .history(&conversation_id, 1, None)
                .ok()
                .and_then(|(page, _)| page.last().map(|message| message.id.local_id.clone()))
        });
    let account = conversation_id.account_id.as_str().to_string();
    let conversation = conversation_id.local_id.clone();
    fire_messaging(
        MessagingCommand::MarkRead {
            conversation_id: conversation_id.clone(),
            message_id: message_id.clone(),
        },
        handover_gmessages::contract::HelperCommand::MarkRead {
            account,
            conversation,
            message: resolved,
        },
        conversation_id,
        writer,
        state,
        messaging,
    )
    .await
}

pub(crate) async fn handle_typing<W>(
    conversation_id: ConversationId,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let account = conversation_id.account_id.as_str().to_string();
    let conversation = conversation_id.local_id.clone();
    fire_messaging(
        MessagingCommand::TypingStart {
            conversation_id: conversation_id.clone(),
        },
        handover_gmessages::contract::HelperCommand::Typing {
            account,
            conversation,
        },
        conversation_id,
        writer,
        state,
        messaging,
    )
    .await
}

pub(crate) async fn handle_delete_message<W>(
    message_id: MessageId,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let account = message_id.conversation_id.account_id.as_str().to_string();
    let conversation = message_id.conversation_id.local_id.clone();
    let message = message_id.local_id.clone();
    request_messaging(
        MessagingCommand::DeleteMessage {
            message_id: message_id.clone(),
        },
        move |request_id| handover_gmessages::contract::HelperCommand::DeleteMessage {
            request_id,
            account,
            conversation,
            message,
        },
        writer,
        state,
        messaging,
    )
    .await
}

pub(crate) async fn handle_open_conversation<W>(
    account_id: MessagingAccountId,
    addresses: Vec<String>,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let account = account_id.as_str().to_string();
    request_messaging(
        MessagingCommand::OpenConversation {
            account_id: account_id.clone(),
            addresses: addresses.clone(),
        },
        move |request_id| handover_gmessages::contract::HelperCommand::OpenConversation {
            request_id,
            account,
            addresses,
        },
        writer,
        state,
        messaging,
    )
    .await
}

pub(crate) async fn handle_sync<W>(
    account_id: MessagingAccountId,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let known = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .messaging()
        .account(&account_id)
        .is_some();
    if !known {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(
                ErrorCode::UnknownMessagingAccount,
                "messaging account is not known",
            ),
        )
        .await?;
        return Ok(true);
    }
    let hub = match require_messaging_hub(messaging) {
        Ok(hub) => hub,
        Err((code, message)) => {
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            return Ok(true);
        }
    };
    match hub
        .fire(handover_gmessages::contract::HelperCommand::Sync {
            account: account_id.as_str().into(),
        })
        .await
    {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::AccountAccepted { account_id }),
            )
            .await?;
            Ok(true)
        }
        Err(error) => {
            let (code, message) = helper_call_error(error);
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            Ok(true)
        }
    }
}

pub(crate) async fn handle_login<W>(
    account_id: MessagingAccountId,
    bundle_b64: String,
    writer: &mut W,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    if account_id.as_str().is_empty() || account_id.as_str().len() > 128 {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(
                ErrorCode::InvalidMessagingCommand,
                "invalid messaging account",
            ),
        )
        .await?;
        return Ok(true);
    }
    if crate::messaging_backend::validate_login_bundle(&bundle_b64).is_err() {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(
                ErrorCode::CredentialRejected,
                "credential bundle was rejected",
            ),
        )
        .await?;
        return Ok(true);
    }
    let hub = match require_messaging_hub(messaging) {
        Ok(hub) => hub,
        Err((code, message)) => {
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            return Ok(true);
        }
    };
    // The bundle travels the local socket and the local helper pipe only.
    // It is never logged, never stored by the daemon, and never argv.
    let account = account_id.as_str().to_string();
    match hub
        .fire(handover_gmessages::contract::HelperCommand::Login {
            account,
            bundle_b64,
        })
        .await
    {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::AccountAccepted { account_id }),
            )
            .await?;
            Ok(true)
        }
        Err(error) => {
            let (code, message) = helper_call_error(error);
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            Ok(true)
        }
    }
}

pub(crate) async fn handle_logout<W>(
    account_id: MessagingAccountId,
    writer: &mut W,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let hub = match require_messaging_hub(messaging) {
        Ok(hub) => hub,
        Err((code, message)) => {
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            return Ok(true);
        }
    };
    // Revocation happens helper-side (remote revoke plus local secret
    // deletion); the daemon drops its copy when AccountRemoved arrives.
    // Acceptance here means the request was queued, not that access is gone.
    match hub
        .fire(handover_gmessages::contract::HelperCommand::Logout {
            account: account_id.as_str().into(),
        })
        .await
    {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::AccountAccepted { account_id }),
            )
            .await?;
            Ok(true)
        }
        Err(error) => {
            let (code, message) = helper_call_error(error);
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            Ok(true)
        }
    }
}

pub(crate) async fn handle_conversations<W>(
    account_id: MessagingAccountId,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let (known, conversations) = {
        let guard = state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            guard.messaging().account(&account_id).is_some(),
            guard
                .messaging()
                .snapshot_conversations()
                .into_iter()
                .filter(|conversation| conversation.id.account_id == account_id)
                .collect::<Vec<_>>(),
        )
    };
    if !known {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(
                ErrorCode::UnknownMessagingAccount,
                "messaging account is not known",
            ),
        )
        .await?;
        return Ok(true);
    }
    let response = ServerMessage::new(ServerPayload::Conversations {
        conversations: conversations.clone(),
    });
    if serde_json::to_vec(&response).map_or(true, |encoded| {
        encoded.len() + 1 > handover_ipc::MAX_LINE_BYTES
    }) {
        write_conversation_chunks(writer, conversations).await?;
    } else {
        write_json_line(writer, &response).await?;
    }
    Ok(true)
}

pub(crate) async fn handle_history<W>(
    conversation_id: ConversationId,
    limit: Option<u32>,
    cursor: Option<String>,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let limit = limit.unwrap_or(25).clamp(1, 100) as usize;
    let read = || {
        state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .messaging()
            .history(&conversation_id, limit, cursor.as_deref())
    };
    match read() {
        Ok((mut messages, mut cursor_next)) => {
            // A successful local read may still be only a bounded live-event
            // window. If the caller asks for more than we currently hold,
            // give the helper one chance to fill the newest page before
            // declaring that history is exhausted. This is what makes a
            // larger "load all" request materially different from rereading
            // the same local window.
            if cursor.is_none() && limit == 100 && messages.len() < limit {
                if let Some(hub) = messaging {
                    if hub
                        .fetch_through_helper(
                            conversation_id.account_id.as_str(),
                            &conversation_id.local_id,
                            limit as u32,
                            None,
                        )
                        .await
                        .is_ok()
                    {
                        if let Ok((fetched_messages, fetched_cursor)) = read() {
                            messages = fetched_messages;
                            cursor_next = fetched_cursor;
                        }
                    }
                }
            }
            let (messages, cursor_next) = fit_history_page(&conversation_id, messages, cursor_next);
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::History {
                    conversation_id,
                    messages,
                    cursor_next,
                }),
            )
            .await?;
            Ok(true)
        }
        Err(HistoryGap::UnknownConversation) => {
            write_json_line(
                writer,
                &ServerMessage::protocol_error(
                    ErrorCode::UnknownConversation,
                    "conversation is not known",
                ),
            )
            .await?;
            Ok(true)
        }
        Err(HistoryGap::CursorOutsideWindow) => {
            // The cursor aged out of the bounded window: page older history
            // through the helper, then serve from the merged window.
            let hub = match require_messaging_hub(messaging) {
                Ok(hub) => hub,
                Err((code, message)) => {
                    write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
                    return Ok(true);
                }
            };
            let fetched = hub
                .fetch_through_helper(
                    conversation_id.account_id.as_str(),
                    &conversation_id.local_id,
                    limit as u32,
                    cursor.clone(),
                )
                .await;
            if fetched.is_err() {
                write_json_line(
                    writer,
                    &ServerMessage::protocol_error(
                        ErrorCode::HistoryUnavailable,
                        "older history is unavailable",
                    ),
                )
                .await?;
                return Ok(true);
            }
            match read() {
                Ok((messages, cursor_next)) => {
                    let (messages, cursor_next) =
                        fit_history_page(&conversation_id, messages, cursor_next);
                    write_json_line(
                        writer,
                        &ServerMessage::new(ServerPayload::History {
                            conversation_id,
                            messages,
                            cursor_next,
                        }),
                    )
                    .await?;
                    Ok(true)
                }
                Err(_) => {
                    write_json_line(
                        writer,
                        &ServerMessage::protocol_error(
                            ErrorCode::HistoryUnavailable,
                            "older history is unavailable",
                        ),
                    )
                    .await?;
                    Ok(true)
                }
            }
        }
    }
}

/// Validate a command, forward it for a `CommandResult` acceptance report,
/// and reply with the acceptance. Acceptance is never delivery.
pub(crate) async fn request_messaging<W>(
    command: MessagingCommand,
    build: impl FnOnce(String) -> handover_gmessages::contract::HelperCommand,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let validation = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .messaging()
        .validate_messaging_command(&command);
    if let Err(error) = validation {
        let (code, message) = messaging_validation_error(&error);
        write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
        return Ok(true);
    }
    let hub = match require_messaging_hub(messaging) {
        Ok(hub) => hub,
        Err((code, message)) => {
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            return Ok(true);
        }
    };
    match hub.request(build).await {
        Ok(outcome) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::MessageAccepted {
                    request_id: outcome.request_id,
                }),
            )
            .await?;
            Ok(true)
        }
        Err(error) => {
            let (code, message) = helper_call_error(error);
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            Ok(true)
        }
    }
}

/// Validate a fire-and-forget command (read receipts, typing pings) and
/// queue it for the helper. The reply confirms daemon acceptance only.
pub(crate) async fn fire_messaging<W>(
    command: MessagingCommand,
    helper: handover_gmessages::contract::HelperCommand,
    conversation_id: ConversationId,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<MessagingHub>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let validation = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .messaging()
        .validate_messaging_command(&command);
    if let Err(error) = validation {
        let (code, message) = messaging_validation_error(&error);
        write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
        return Ok(true);
    }
    let hub = match require_messaging_hub(messaging) {
        Ok(hub) => hub,
        Err((code, message)) => {
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            return Ok(true);
        }
    };
    match hub.fire(helper).await {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::ConversationAccepted { conversation_id }),
            )
            .await?;
            Ok(true)
        }
        Err(error) => {
            let (code, message) = helper_call_error(error);
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use handover_core::{
        BatteryState, Capability, Device, DeviceEvent, DeviceId, MediaCommand, MediaControl,
        MediaEvent, MediaSession, MediaSessionId, Message, MessageId, MessagingAccountId,
        Notification, NotificationEvent, NotificationId, PlaybackState,
    };
    use handover_ipc::{Client, ServerPayload};
    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};

    use super::*;

    #[test]
    fn oversized_history_page_is_cut_newest_first() {
        use handover_core::{ConversationId, Participant};
        let conversation = ConversationId::new(MessagingAccountId::new("a"), "c");
        // 100 legal messages at maximum text size would exceed the
        // 1 MiB line limit several times over.
        let messages: Vec<Message> = (0..100)
            .map(|index| Message {
                id: MessageId::new(conversation.clone(), format!("m{index:03}")),
                sender: Participant {
                    local_id: "peer".into(),
                    display_name: None,
                    address: None,
                    is_self: false,
                },
                transport: None,
                sent_at: Some(index),
                text: Some("x".repeat(8000)),
                attachments: vec![],
                reply_to: None,
                reactions: vec![],
                deleted: false,
            })
            .collect();
        let (kept, cursor) = fit_history_page(&conversation, messages, None);
        assert!(!kept.is_empty());
        assert!(kept.len() < 100);
        let encoded = serde_json::to_vec(&ServerMessage::new(ServerPayload::History {
            conversation_id: conversation,
            messages: kept.clone(),
            cursor_next: cursor.clone(),
        }))
        .expect("serializes");
        assert!(encoded.len() <= MAX_HISTORY_LINE_BYTES);
        // The cursor addresses the oldest kept message, so the next
        // page overlaps the cut instead of skipping it.
        assert_eq!(
            cursor.as_deref(),
            kept.first().map(|message| message.id.local_id.as_str())
        );
        assert_eq!(kept.last().map(|m| m.id.local_id.as_str()), Some("m099"));
    }

    #[tokio::test]
    async fn oversized_snapshot_chunks_every_collection() {
        let notifications = (0..2)
            .map(|index| Notification {
                id: NotificationId::new(DeviceId::new("phone"), index.to_string()),
                app_name: "Messages".into(),
                title: "Large".into(),
                body: "x".repeat(700 * 1024),
                icon_path: None,
                clearable: true,
                actions: vec![],
                reply_supported: false,
            })
            .collect();
        let (mut reader, mut writer) = tokio::io::duplex(4 * 1024 * 1024);
        write_snapshot_chunks(
            &mut writer,
            vec![],
            notifications,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        )
        .await
        .expect("snapshot chunks fit");
        drop(writer);

        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.expect("read chunks");
        let lines = bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty());
        let mut total_notifications = 0;
        for line in lines {
            assert!(line.len() < handover_ipc::MAX_LINE_BYTES);
            let message: ServerMessage = serde_json::from_slice(line).expect("valid chunk");
            if let ServerPayload::SnapshotChunk { notifications, .. } = message.payload {
                total_notifications += notifications.len();
            }
        }
        assert_eq!(total_notifications, 2);
    }

    fn device(name: &str, percentage: u8) -> Device {
        Device {
            id: DeviceId::new(name.to_lowercase()),
            name: name.into(),
            connected: true,
            paired: true,
            battery: Some(BatteryState::new(percentage, false).expect("valid battery")),
            connectivity: None,
            capabilities: BTreeSet::from([Capability::Battery]),
        }
    }

    #[tokio::test]
    async fn pre_subscription_reads_have_a_bounded_idle_timeout() {
        let (_client, server) = tokio::io::duplex(64);
        let mut reader = BufReader::new(server);
        assert!(
            read_pre_subscription_request(&mut reader, Duration::from_millis(1))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn calls_snapshot_and_rejections_use_isolated_ipc() {
        use handover_core::{CallAction, CallEvent, CallPhase, CallState};
        let (_directory, path, state, _events, task) = server_with_device().await;
        // No native backend or real phone exists in this fixture.
        let call = CallState {
            device_id: DeviceId::new("phone"),
            phase: CallPhase::Idle,
            controls: BTreeSet::from([CallAction::Place]),
            generation: 1,
        };
        state
            .write()
            .unwrap()
            .apply(StateEvent::Call(CallEvent::Updated(call.clone())));
        let mut client = Client::connect_to(path.clone()).await.unwrap();
        assert_eq!(client.calls().await.unwrap(), vec![call.clone()]);
        assert!(
            client
                .call_control(call.device_id.clone(), CallAction::Answer, None)
                .await
                .is_err()
        );
        assert!(
            client
                .call_control(
                    call.device_id.clone(),
                    CallAction::Place,
                    Some("*#06#".into())
                )
                .await
                .is_err()
        );
        let subscription = client.subscribe().await.unwrap();
        assert_eq!(subscription.calls, vec![call]);
        task.abort();
    }

    fn notification() -> Notification {
        Notification {
            id: NotificationId::new(DeviceId::new("phone"), "1"),
            app_name: "Messages".into(),
            title: "Alice".into(),
            body: "Hello".into(),
            icon_path: None,
            clearable: true,
            actions: Vec::new(),
            reply_supported: true,
        }
    }

    fn media_session() -> MediaSession {
        MediaSession {
            id: MediaSessionId::new(DeviceId::new("phone"), "Player"),
            application: "Player".into(),
            title: Some("Test song".into()),
            artist: None,
            album: None,
            playback: PlaybackState::Paused,
            position_ms: Some(0),
            duration_ms: None,
            volume_percent: None,
            controls: BTreeSet::from([MediaControl::Play]),
        }
    }

    #[tokio::test]
    async fn media_snapshot_subscription_and_events_are_consistent() {
        let (_directory, path, state, events, task) = server_with_device().await;
        let session = media_session();
        state
            .write()
            .unwrap()
            .apply(StateEvent::Media(MediaEvent::Added(session.clone())));
        let mut client = Client::connect_to(path.clone()).await.expect("connect");
        assert_eq!(
            client.media_sessions().await.expect("list"),
            vec![session.clone()]
        );
        let client = Client::connect_to(path).await.expect("connect");
        let mut subscription = client.subscribe().await.expect("subscribe");
        assert_eq!(subscription.media_sessions, vec![session.clone()]);
        let mut updated = session;
        updated.playback = PlaybackState::Playing;
        state
            .write()
            .unwrap()
            .apply(StateEvent::Media(MediaEvent::Updated(updated.clone())));
        events
            .send(StateEvent::Media(MediaEvent::Updated(updated.clone())))
            .expect("subscriber");
        assert_eq!(
            subscription.next_message().await.expect("event").payload,
            ServerPayload::MediaUpdated {
                media_session: updated
            }
        );
        task.abort();
    }

    #[tokio::test]
    async fn media_preflight_rejects_unknown_and_unsupported_controls() {
        let (_directory, path, state, _events, task) = server_with_device().await;
        let mut client = Client::connect_to(path).await.expect("connect");
        let id = media_session().id;
        assert!(matches!(
            client
                .media_command(MediaCommand::Play { id: id.clone() })
                .await,
            Err(IpcError::Server {
                code: ErrorCode::MediaSessionNotFound,
                ..
            })
        ));
        state
            .write()
            .unwrap()
            .apply(StateEvent::Media(MediaEvent::Added(media_session())));
        assert!(matches!(
            client.media_command(MediaCommand::Pause { id }).await,
            Err(IpcError::Server {
                code: ErrorCode::InvalidMediaCommand,
                ..
            })
        ));
        task.abort();
    }

    async fn server_with_device() -> (
        TempDir,
        PathBuf,
        Arc<RwLock<StateStore>>,
        broadcast::Sender<StateEvent>,
        tokio::task::JoinHandle<()>,
    ) {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("handoverd.sock");
        let mut store = StateStore::default();
        store.apply(StateEvent::Device(DeviceEvent::Added(device("Phone", 72))));
        let state = Arc::new(RwLock::new(store));
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let server = IpcServer::bind_at(path.clone(), Arc::clone(&state), events.clone(), None)
            .await
            .expect("server binds");
        let task = tokio::spawn(async move {
            server.run().await.expect("server runs");
        });
        (directory, path, state, events, task)
    }

    #[tokio::test]
    async fn serves_device_snapshot() {
        let (_directory, path, _state, _events, task) = server_with_device().await;
        let mut client = Client::connect_to(path).await.expect("client connects");

        let devices = client.devices().await.expect("snapshot succeeds");

        assert_eq!(devices, vec![device("Phone", 72)]);
        task.abort();
    }

    #[tokio::test]
    async fn subscription_receives_device_event() {
        let (_directory, path, state, events, task) = server_with_device().await;
        let client = Client::connect_to(path).await.expect("client connects");
        let mut subscription = client.subscribe().await.expect("subscription succeeds");
        let updated = device("Phone", 71);
        state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .apply(StateEvent::Device(DeviceEvent::Updated(updated.clone())));

        events
            .send(StateEvent::Device(DeviceEvent::Updated(updated.clone())))
            .expect("subscriber exists");
        let message = subscription.next_message().await.expect("event arrives");

        assert_eq!(
            message.payload,
            ServerPayload::DeviceUpdated { device: updated }
        );
        task.abort();
    }

    #[tokio::test]
    async fn serves_multiple_clients_after_disconnect() {
        let (_directory, path, _state, _events, task) = server_with_device().await;
        let first = Client::connect_to(path.clone())
            .await
            .expect("first client connects");
        let mut second = Client::connect_to(path)
            .await
            .expect("second client connects");
        drop(first);

        assert_eq!(
            second.devices().await.expect("second client works"),
            vec![device("Phone", 72)]
        );
        task.abort();
    }

    #[tokio::test]
    async fn native_notification_commands_route_away_from_kde() {
        use handover_core::{Notification, NotificationId};
        let (_directory, path, state, _events, task) = server_with_device().await;
        let native = Notification {
            id: NotificationId::new(handover_core::DeviceId::new("native:cert"), "key-1"),
            app_name: "Example".into(),
            title: "Hello".into(),
            body: "World".into(),
            icon_path: None,
            clearable: true,
            actions: vec![],
            reply_supported: false,
        };
        state
            .write()
            .unwrap()
            .apply(StateEvent::Notification(NotificationEvent::Added(
                native.clone(),
            )));
        let mut client = Client::connect_to(path).await.expect("connect");
        // No native backend is running in this test, so routing must report
        // an unavailable native path rather than attempting a KDE D-Bus call
        // for a `native:` device.
        let error = client
            .dismiss_notification(native.id)
            .await
            .expect_err("native backend absent");
        assert!(matches!(
            error,
            IpcError::Server {
                code: ErrorCode::BackendUnavailable,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn native_media_commands_route_away_from_kde() {
        use handover_core::{MediaSession, MediaSessionId, PlaybackState};
        let (_directory, path, state, _events, task) = server_with_device().await;
        // The fixture device lacks the media capability, so add a capable
        // native device first: routing is checked after validation passes.
        let device = Device {
            id: DeviceId::new("native:cert"),
            name: "Native Phone".into(),
            connected: true,
            paired: true,
            battery: None,
            connectivity: None,
            capabilities: BTreeSet::from([
                handover_core::Capability::Battery,
                handover_core::Capability::Media,
            ]),
        };
        state
            .write()
            .unwrap()
            .apply(StateEvent::Device(DeviceEvent::Added(device.clone())));
        let session = MediaSession {
            id: MediaSessionId::new(device.id.clone(), "com.example.music"),
            application: "Example Music".into(),
            title: Some("Test track".into()),
            artist: None,
            album: None,
            playback: PlaybackState::Playing,
            position_ms: None,
            duration_ms: None,
            volume_percent: None,
            controls: BTreeSet::from([handover_core::MediaControl::Pause]),
        };
        state
            .write()
            .unwrap()
            .apply(StateEvent::Media(MediaEvent::Added(session.clone())));
        let mut client = Client::connect_to(path).await.expect("connect");
        // No native backend is running in this test, so routing must report
        // an unavailable native path rather than attempting a KDE D-Bus call
        // for a `native:` session.
        let error = client
            .media_command(MediaCommand::Pause { id: session.id })
            .await
            .expect_err("native backend absent");
        assert!(matches!(
            error,
            IpcError::Server {
                code: ErrorCode::BackendUnavailable,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn notification_snapshot_and_subscription_are_consistent() {
        let (_directory, path, state, events, task) = server_with_device().await;
        let current = notification();
        state
            .write()
            .unwrap()
            .apply(StateEvent::Notification(NotificationEvent::Added(
                current.clone(),
            )));
        let mut client = Client::connect_to(path.clone()).await.expect("connect");
        assert_eq!(
            client.notifications().await.expect("list"),
            vec![current.clone()]
        );

        let client = Client::connect_to(path).await.expect("connect");
        let mut subscription = client.subscribe().await.expect("subscribe");
        assert_eq!(subscription.notifications, vec![current.clone()]);

        let mut updated = current.clone();
        updated.title = "Alice again".into();
        state
            .write()
            .unwrap()
            .apply(StateEvent::Notification(NotificationEvent::Updated(
                updated.clone(),
            )));
        events
            .send(StateEvent::Notification(NotificationEvent::Updated(
                updated.clone(),
            )))
            .expect("subscriber");
        assert_eq!(
            subscription.next_message().await.expect("event").payload,
            ServerPayload::NotificationUpdated {
                notification: updated
            }
        );
        task.abort();
    }

    #[tokio::test]
    async fn rejects_unknown_notification_and_action_without_dbus() {
        let (_directory, path, state, _events, task) = server_with_device().await;
        let mut client = Client::connect_to(path).await.expect("connect");
        let error = client
            .dismiss_notification(notification().id.clone())
            .await
            .expect_err("unknown notification");
        assert!(matches!(
            error,
            IpcError::Server {
                code: ErrorCode::NotificationNotFound,
                ..
            }
        ));

        let current = notification();
        state
            .write()
            .unwrap()
            .apply(StateEvent::Notification(NotificationEvent::Added(
                current.clone(),
            )));
        let error = client
            .invoke_notification_action(current.id, "unknown".into())
            .await
            .expect_err("unknown action");
        assert!(matches!(
            error,
            IpcError::Server {
                code: ErrorCode::InvalidNotificationCommand,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn lag_recovery_snapshot_contains_both_state_maps() {
        let (_directory, _path, state, _events, task) = server_with_device().await;
        let current = notification();
        state
            .write()
            .unwrap()
            .apply(StateEvent::Notification(NotificationEvent::Added(
                current.clone(),
            )));

        let media = media_session();
        state
            .write()
            .unwrap()
            .apply(StateEvent::Media(MediaEvent::Added(media.clone())));

        assert_eq!(
            snapshot_payload(&state, true, false),
            ServerPayload::Snapshot {
                devices: vec![device("Phone", 72)],
                notifications: vec![current],
                media_sessions: vec![media],
                calls: vec![],
                messaging_accounts: vec![],
                conversations: vec![],
                typing_states: vec![],
                read_states: vec![],
            }
        );
        task.abort();
    }

    #[tokio::test]
    async fn malformed_input_gets_controlled_error() {
        let (_directory, path, _state, _events, task) = server_with_device().await;
        let mut stream = UnixStream::connect(path)
            .await
            .expect("raw client connects");
        stream
            .write_all(b"this is not json\n")
            .await
            .expect("write succeeds");
        let (reader, _writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        let response: ServerMessage = read_json_line(&mut reader)
            .await
            .expect("read succeeds")
            .expect("response exists");

        assert!(matches!(
            response.payload,
            ServerPayload::Error {
                code: ErrorCode::MalformedRequest,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn unsupported_protocol_gets_controlled_error() {
        let (_directory, path, _state, _events, task) = server_with_device().await;
        let mut stream = UnixStream::connect(path)
            .await
            .expect("raw client connects");
        stream
            .write_all(b"{\"protocol\":2,\"method\":\"hello\"}\n")
            .await
            .expect("write succeeds");
        let (reader, _writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        let response: ServerMessage = read_json_line(&mut reader)
            .await
            .expect("read succeeds")
            .expect("response exists");

        assert!(matches!(
            response.payload,
            ServerPayload::Error {
                code: ErrorCode::UnsupportedProtocol,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn bounded_event_channel_reports_lag() {
        let (events, mut receiver) = broadcast::channel(2);
        for percentage in [70, 69, 68] {
            events
                .send(StateEvent::Device(DeviceEvent::Updated(device(
                    "Phone", percentage,
                ))))
                .expect("receiver exists");
        }

        assert!(matches!(
            receiver.recv().await,
            Err(broadcast::error::RecvError::Lagged(1))
        ));
    }

    #[test]
    fn outgoing_url_validation_accepts_real_urls_and_rejects_unsafe_values() {
        assert_eq!(
            validate_outgoing_url("https://example.com/a?q=1"),
            Ok("https://example.com/a?q=1".into())
        );
        assert!(validate_outgoing_url("mailto:someone@example.com").is_ok());
        for input in [
            "",
            "example.com",
            " https://example.com",
            "javascript:alert(1)",
            "file:///tmp/x",
            "https://example.com\n",
        ] {
            assert_eq!(
                validate_outgoing_url(input),
                Err(ErrorCode::InvalidResource)
            );
        }
    }

    #[tokio::test]
    async fn file_validation_accepts_regular_unicode_file_and_rejects_other_paths() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("handover ✓ test.txt");
        tokio::fs::write(&path, b"harmless test content")
            .await
            .expect("test file");
        let file_url = Url::from_file_path(&path).expect("file URL").to_string();
        assert_eq!(validate_file_url(&file_url).await, Ok(file_url.clone()));
        let link = directory.path().join("linked file.txt");
        std::os::unix::fs::symlink(&path, &link).expect("create test symlink");
        let link_url = Url::from_file_path(&link).expect("file URL").to_string();
        assert_eq!(validate_file_url(&link_url).await, Ok(link_url));
        let missing = Url::from_file_path(directory.path().join("missing.txt"))
            .expect("file URL")
            .to_string();
        assert_eq!(
            validate_file_url(&missing).await,
            Err(ErrorCode::ResourceNotFound)
        );
        assert_eq!(
            validate_file_url(Url::from_file_path(directory.path()).unwrap().as_ref()).await,
            Err(ErrorCode::InvalidResource)
        );
        assert_eq!(
            validate_file_url(&format!("{file_url}?unexpected=1")).await,
            Err(ErrorCode::InvalidResource)
        );
    }

    #[tokio::test]
    async fn share_preflight_rejects_unknown_device_and_unsupported_plugin() {
        let (_directory, path, _state, _events, task) = server_with_device().await;
        let mut client = Client::connect_to(path).await.expect("client connects");
        let unknown = client
            .send_url(DeviceId::new("unknown"), "https://example.com".into())
            .await
            .expect_err("unknown target");
        assert!(matches!(
            unknown,
            IpcError::Server {
                code: ErrorCode::UnknownDevice,
                ..
            }
        ));
        let unsupported = client
            .send_url(DeviceId::new("phone"), "https://example.com".into())
            .await
            .expect_err("unsupported plugin");
        assert!(matches!(
            unsupported,
            IpcError::Server {
                code: ErrorCode::UnsupportedCapability,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn share_preflight_rejects_missing_file_before_dbus() {
        let (_directory, path, state, _events, task) = server_with_device().await;
        let mut capable = device("Phone", 72);
        capable.capabilities.insert(Capability::FileTransfer);
        state
            .write()
            .unwrap()
            .apply(StateEvent::Device(DeviceEvent::Updated(capable)));
        let mut client = Client::connect_to(path).await.expect("client connects");
        let missing =
            Url::from_file_path("/tmp/handover-definitely-missing-test.txt").expect("file URL");
        let error = client
            .send_file_url(DeviceId::new("phone"), missing.into())
            .await
            .expect_err("missing file");
        assert!(matches!(
            error,
            IpcError::Server {
                code: ErrorCode::ResourceNotFound,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn share_preflight_rejects_disconnected_unpaired_and_invalid_url() {
        let (_directory, path, state, _events, task) = server_with_device().await;
        let mut client = Client::connect_to(path).await.expect("client connects");
        let mut target = device("Phone", 72);
        target.capabilities.insert(Capability::FileTransfer);
        target.connected = false;
        state
            .write()
            .unwrap()
            .apply(StateEvent::Device(DeviceEvent::Updated(target.clone())));
        assert!(matches!(
            client
                .send_url(target.id.clone(), "https://example.com".into())
                .await,
            Err(IpcError::Server {
                code: ErrorCode::DeviceDisconnected,
                ..
            })
        ));

        target.connected = true;
        target.paired = false;
        state
            .write()
            .unwrap()
            .apply(StateEvent::Device(DeviceEvent::Updated(target.clone())));
        assert!(matches!(
            client
                .send_url(target.id.clone(), "https://example.com".into())
                .await,
            Err(IpcError::Server {
                code: ErrorCode::DeviceNotPaired,
                ..
            })
        ));

        target.paired = true;
        state
            .write()
            .unwrap()
            .apply(StateEvent::Device(DeviceEvent::Updated(target.clone())));
        assert!(matches!(
            client.send_url(target.id, "not a URL".into()).await,
            Err(IpcError::Server {
                code: ErrorCode::InvalidResource,
                ..
            })
        ));
        assert!(matches!(
            client
                .send_url(DeviceId::new("phone"), "not a URL".into())
                .await,
            Err(IpcError::Server {
                code: ErrorCode::InvalidResource,
                ..
            })
        ));
        assert_eq!(
            client
                .devices()
                .await
                .expect("connection stays usable")
                .len(),
            1
        );
        task.abort();
    }

    #[tokio::test]
    async fn incoming_share_reaches_subscriber_without_entering_snapshot() {
        let (_directory, path, state, events, task) = server_with_device().await;
        let client = Client::connect_to(path).await.expect("client connects");
        let mut subscription = client.subscribe().await.expect("subscription succeeds");
        let share = handover_core::ReceivedShare {
            device_id: DeviceId::new("phone"),
            resource: handover_core::SharedResource::Url {
                url: "https://example.com".into(),
            },
        };
        let event = StateEvent::ShareReceived(share.clone());
        state.write().unwrap().apply(event.clone());
        events.send(event).expect("subscriber exists");
        assert_eq!(
            subscription
                .next_message()
                .await
                .expect("share event")
                .payload,
            ServerPayload::ShareReceived { share }
        );
        assert!(state.read().unwrap().snapshot().notifications.is_empty());
        task.abort();
    }

    #[tokio::test]
    async fn legacy_subscriber_does_not_receive_new_share_event() {
        let (_directory, path, _state, events, task) = server_with_device().await;
        let mut stream = UnixStream::connect(path).await.expect("connect");
        stream
            .write_all(b"{\"protocol\":1,\"method\":\"subscribe\"}\n")
            .await
            .expect("subscribe");
        let (reader, _writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let initial: ServerMessage = read_json_line(&mut reader)
            .await
            .expect("read")
            .expect("response");
        assert!(matches!(initial.payload, ServerPayload::Subscribed { .. }));
        events
            .send(StateEvent::ShareReceived(handover_core::ReceivedShare {
                device_id: DeviceId::new("phone"),
                resource: handover_core::SharedResource::Url {
                    url: "https://example.com".into(),
                },
            }))
            .expect("subscriber");
        events
            .send(StateEvent::Device(DeviceEvent::Updated(device(
                "Phone", 71,
            ))))
            .expect("subscriber");
        let next: ServerMessage = read_json_line(&mut reader)
            .await
            .expect("read")
            .expect("event");
        assert!(matches!(next.payload, ServerPayload::DeviceUpdated { .. }));
        task.abort();
    }

    #[tokio::test]
    async fn legacy_subscriber_does_not_receive_media_events() {
        let (_directory, path, _state, events, task) = server_with_device().await;
        let mut stream = UnixStream::connect(path).await.expect("connect");
        stream
            .write_all(b"{\"protocol\":1,\"method\":\"subscribe\"}\n")
            .await
            .expect("subscribe");
        let (reader, _writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let initial: ServerMessage = read_json_line(&mut reader)
            .await
            .expect("read")
            .expect("response");
        assert!(matches!(initial.payload, ServerPayload::Subscribed { .. }));
        events
            .send(StateEvent::Media(MediaEvent::Added(media_session())))
            .expect("subscriber");
        events
            .send(StateEvent::Device(DeviceEvent::Updated(device(
                "Phone", 70,
            ))))
            .expect("subscriber");
        let next: ServerMessage = read_json_line(&mut reader)
            .await
            .expect("read")
            .expect("event");
        assert!(matches!(next.payload, ServerPayload::DeviceUpdated { .. }));
        task.abort();
    }
}

#[cfg(test)]
mod messaging_live_tests {
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};
    use std::time::{Duration, Instant};

    use handover_core::{ConversationId, MessagingAccountId};
    use handover_ipc::{Client, ErrorCode, IpcError, ServerPayload};
    use tokio::time::sleep;

    use super::*;
    use crate::messaging_backend::{MessagingHub, spawn_supervisor};

    fn live_device(name: &str, percentage: u8) -> handover_core::Device {
        handover_core::Device {
            id: handover_core::DeviceId::new(name.to_lowercase()),
            name: name.into(),
            connected: true,
            paired: true,
            battery: Some(
                handover_core::BatteryState::new(percentage, false).expect("valid battery"),
            ),
            connectivity: None,
            capabilities: std::collections::BTreeSet::from([handover_core::Capability::Battery]),
        }
    }

    fn helper_binary() -> Option<PathBuf> {
        for candidate in [
            PathBuf::from("../target/debug/handover-gmessages-helper"),
            PathBuf::from("target/debug/handover-gmessages-helper"),
        ] {
            if candidate.is_file() {
                return candidate.canonicalize().ok();
            }
        }
        None
    }

    async fn wait_until(mut condition: impl FnMut() -> bool, timeout: Duration, what: &str) {
        let start = Instant::now();
        while !condition() {
            if start.elapsed() > timeout {
                panic!("timed out waiting for {what}");
            }
            sleep(Duration::from_millis(50)).await;
        }
    }

    /// Full live walkthrough against the in-memory loopback helper: login,
    /// seeded SMS/RCS/group conversations, paged history, send, attachment,
    /// reply, reactions, typing, read state, delivery progression, open,
    /// delete, restart recovery, isolation, and logout.
    #[tokio::test]
    async fn messaging_end_to_end_through_loopback_helper() {
        let Some(helper) = helper_binary() else {
            eprintln!("skipping live messaging test: helper binary not built");
            return;
        };
        let state_home = tempfile::tempdir().expect("state home");
        // Test-only env mutation. No other test in this binary reads these
        // variables, and the helper child inherits them for secret storage.
        unsafe {
            std::env::set_var("HANDOVER_GMESSAGES_HELPER", &helper);
            std::env::set_var("XDG_STATE_HOME", state_home.path());
        }

        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("handoverd.sock");
        let mut store = StateStore::default();
        store.apply(StateEvent::Device(DeviceEvent::Added(live_device(
            "Phone", 72,
        ))));
        let state = Arc::new(RwLock::new(store));
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let hub = MessagingHub::new();
        let supervisor = spawn_supervisor(Arc::clone(&state), events.clone(), hub.clone());
        let server = IpcServer::bind_at(
            path.clone(),
            Arc::clone(&state),
            events.clone(),
            Some(hub.clone()),
        )
        .await
        .expect("server binds");
        let task = tokio::spawn(async move {
            server.run().await.expect("server runs");
        });

        let mut client = Client::connect_to(path.clone()).await.expect("connect");
        let subscriber = Client::connect_to(path).await.expect("connect");
        let mut subscription = subscriber.subscribe().await.expect("subscribe");

        // The supervisor spawns asynchronously; wait for the helper link.
        wait_until(
            || hub.try_has_sender(),
            Duration::from_secs(10),
            "helper link",
        )
        .await;

        // Empty bundles are rejected before touching the helper.
        let login_error = client
            .messaging_login(MessagingAccountId::new("gmessages:test"), String::new())
            .await
            .expect_err("empty bundle");
        assert!(matches!(
            login_error,
            IpcError::Server {
                code: ErrorCode::CredentialRejected,
                ..
            }
        ));

        // Login queues the bundle with the helper; pairing completes on sync.
        client
            .messaging_login(MessagingAccountId::new("gmessages:test"), "dGVzdA==".into())
            .await
            .expect("login accepted");
        wait_until(
            || {
                state
                    .read()
                    .unwrap()
                    .messaging()
                    .account(&MessagingAccountId::new("gmessages:test"))
                    .is_some_and(|account| account.authenticated)
            },
            Duration::from_secs(15),
            "authenticated account",
        )
        .await;

        // Seeded SMS, RCS, and group conversations load.
        let conversations = client
            .messaging_conversations(MessagingAccountId::new("gmessages:test"))
            .await
            .expect("conversations");
        assert_eq!(conversations.len(), 3);
        let rcs = conversations
            .iter()
            .find(|conversation| conversation.id.local_id == "thread-rcs")
            .expect("rcs thread");
        assert_eq!(rcs.transport, handover_core::TransportKind::Rcs);
        assert!(
            rcs.capabilities
                .contains(&handover_core::MessagingCapability::Text)
        );
        let group = conversations
            .iter()
            .find(|conversation| conversation.id.local_id == "thread-group")
            .expect("group thread");
        assert_eq!(group.kind, handover_core::ConversationKind::Group);
        assert_eq!(group.participants.len(), 3);

        // Paged history: newest page plus an older cursor page.
        let (page, next) = client
            .messaging_history(rcs.id.clone(), Some(2), None)
            .await
            .expect("history");
        assert_eq!(page.len(), 2);
        let next = next.expect("older page exists");
        let (older, end) = client
            .messaging_history(rcs.id.clone(), Some(10), Some(next))
            .await
            .expect("older history");
        assert!(!older.is_empty());
        assert!(end.is_none());

        // Unknown conversations stay unknown; bogus cursors report a gap.
        let unknown = client
            .messaging_history(
                ConversationId::new(MessagingAccountId::new("gmessages:test"), "missing"),
                None,
                None,
            )
            .await
            .expect_err("unknown conversation");
        assert!(matches!(
            unknown,
            IpcError::Server {
                code: ErrorCode::UnknownConversation,
                ..
            }
        ));
        let gap = client
            .messaging_history(rcs.id.clone(), None, Some("missing-cursor".into()))
            .await
            .expect_err("cursor gap");
        assert!(matches!(
            gap,
            IpcError::Server {
                code: ErrorCode::HistoryUnavailable,
                ..
            }
        ));

        // Reactions need an attested capability: the SMS thread has none.
        let sms = conversations
            .iter()
            .find(|conversation| conversation.id.local_id == "thread-sms")
            .expect("sms thread");
        let (sms_history, _) = client
            .messaging_history(sms.id.clone(), Some(1), None)
            .await
            .expect("sms history");
        let sms_message = sms_history.first().expect("sms message").id.clone();
        let react_error = client
            .react_to_message(sms_message, "❤".into())
            .await
            .expect_err("sms reactions unattested");
        assert!(matches!(
            react_error,
            IpcError::Server {
                code: ErrorCode::UnsupportedMessagingCapability,
                ..
            }
        ));

        // Send text: accepted now, displayed later, never assumed.
        let request_id = client
            .send_message_text(rcs.id.clone(), "hello from the live test".into())
            .await
            .expect("send accepted");
        assert!(request_id.starts_with("msgreq-"));
        // Drain the attested pipeline one stage per sync.
        for _ in 0..3 {
            client
                .messaging_sync(MessagingAccountId::new("gmessages:test"))
                .await
                .expect("sync");
        }
        let displayed = wait_for_status(&mut subscription, "displayed").await;
        assert!(displayed.starts_with("gmessages:test:thread-rcs:out-"));

        // The sent message is visible with self attribution.
        let (fresh, _) = client
            .messaging_history(rcs.id.clone(), Some(10), None)
            .await
            .expect("fresh history");
        let own = fresh
            .iter()
            .find(|message| {
                message.sender.is_self
                    && message.text.as_deref() == Some("hello from the live test")
            })
            .expect("own message stored")
            .id
            .clone();

        // Reply sends into the same thread (validated by the daemon).
        client
            .send_message_text(rcs.id.clone(), "a reply in the same thread".into())
            .await
            .expect("reply accepted");

        // Attachment round trip with a real staged file.
        let attachment_dir = tempfile::tempdir().expect("attachment dir");
        let attachment_path = attachment_dir.path().join("live ✓.bin");
        tokio::fs::write(&attachment_path, b"live attachment bytes")
            .await
            .expect("write attachment");
        let file_url = url::Url::from_file_path(&attachment_path)
            .expect("file url")
            .to_string();
        client
            .send_message_file(rcs.id.clone(), file_url, Some("caption".into()))
            .await
            .expect("attachment accepted");
        wait_for_attachment(&mut subscription, "live ✓.bin").await;
        let (with_file, _) = client
            .messaging_history(rcs.id.clone(), Some(10), None)
            .await
            .expect("history with file");
        let staged = with_file
            .iter()
            .find(|message| {
                message
                    .attachments
                    .iter()
                    .any(|attachment| attachment.name.as_deref() == Some("live ✓.bin"))
            })
            .expect("attachment stored");
        let staged_path = staged.attachments[0]
            .staged_path
            .as_deref()
            .expect("helper returned a confined staged path");
        assert!(staged_path.contains("/handover/gmessages/imported/"));
        assert_ne!(staged_path, attachment_path.to_str().unwrap());

        // Reactions add and remove.
        client
            .react_to_message(own.clone(), "👍".into())
            .await
            .expect("react accepted");
        wait_for_reaction(&mut subscription, &own, "👍", true).await;
        let (reacted, _) = client
            .messaging_history(rcs.id.clone(), Some(10), None)
            .await
            .expect("reacted history");
        let entry = reacted
            .iter()
            .find(|message| message.id == own)
            .expect("own message");
        assert!(
            entry
                .reactions
                .iter()
                .any(|reaction| reaction.emoji == "👍")
        );
        client
            .unreact_to_message(own.clone(), "👍".into())
            .await
            .expect("unreact accepted");
        wait_for_reaction(&mut subscription, &own, "👍", false).await;
        let (unreacted, _) = client
            .messaging_history(rcs.id.clone(), Some(10), None)
            .await
            .expect("unreacted history");
        let entry = unreacted
            .iter()
            .find(|message| message.id == own)
            .expect("own message");
        assert!(entry.reactions.is_empty());

        // Only own messages can be deleted; peer messages are rejected.
        let peer = fresh
            .iter()
            .find(|message| !message.sender.is_self)
            .expect("peer message")
            .id
            .clone();
        let delete_error = client
            .delete_message(peer)
            .await
            .expect_err("peer delete rejected");
        assert!(matches!(
            delete_error,
            IpcError::Server {
                code: ErrorCode::InvalidMessagingCommand,
                ..
            }
        ));
        client
            .delete_message(own.clone())
            .await
            .expect("delete accepted");
        wait_for_message_removed(&mut subscription, &own).await;
        let (after_delete, _) = client
            .messaging_history(rcs.id.clone(), Some(20), None)
            .await
            .expect("history after delete");
        assert!(!after_delete.iter().any(|message| message.id == own));

        // Typing-start queues; the helper reports inbound typing.
        client
            .start_typing(rcs.id.clone())
            .await
            .expect("typing accepted");
        wait_for_typing(&mut subscription, &rcs.id).await;

        // Mark read clears the unread flag.
        client
            .mark_conversation_read(rcs.id.clone(), None)
            .await
            .expect("read accepted");
        wait_until(
            || {
                state
                    .read()
                    .unwrap()
                    .messaging()
                    .snapshot_read()
                    .iter()
                    .any(|read| read.conversation_id == rcs.id && !read.unread)
            },
            Duration::from_secs(10),
            "read state",
        )
        .await;

        // Open a new conversation by address.
        client
            .open_conversation(
                MessagingAccountId::new("gmessages:test"),
                vec!["+15559999".into()],
            )
            .await
            .expect("open accepted");
        wait_until(
            || {
                state
                    .read()
                    .unwrap()
                    .messaging()
                    .snapshot_conversations()
                    .len()
                    == 4
            },
            Duration::from_secs(10),
            "opened conversation",
        )
        .await;

        // Helper traffic never disturbs device state.
        assert_eq!(client.devices().await.expect("devices").len(), 1);

        // Logout revokes access: the account and its state disappear.
        client
            .messaging_logout(MessagingAccountId::new("gmessages:test"))
            .await
            .expect("logout accepted");
        wait_until(
            || {
                state
                    .read()
                    .unwrap()
                    .messaging()
                    .snapshot_accounts()
                    .is_empty()
            },
            Duration::from_secs(10),
            "account removal",
        )
        .await;
        assert!(
            state
                .read()
                .unwrap()
                .messaging()
                .snapshot_conversations()
                .is_empty()
        );

        // Shut the helper down so no orphan process outlives the test:
        // aborting the supervisor drops the child (kill-on-drop).
        supervisor.abort();
        task.abort();
        unsafe {
            std::env::remove_var("HANDOVER_GMESSAGES_HELPER");
            std::env::remove_var("XDG_STATE_HOME");
        }
    }

    async fn wait_for_status(
        subscription: &mut handover_ipc::Subscription,
        status: &str,
    ) -> String {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if Instant::now() > deadline {
                panic!("timed out waiting for status {status}");
            }
            let message = tokio::time::timeout(Duration::from_secs(5), subscription.next_message())
                .await
                .expect("event in time")
                .expect("event decodes");
            if let ServerPayload::MessageStatus { update } = message.payload {
                let rendered = format!("{:?}", update.status).to_lowercase();
                if rendered.contains(status) {
                    return update.message_id.to_string();
                }
            }
        }
    }

    async fn wait_for_reaction(
        subscription: &mut handover_ipc::Subscription,
        message_id: &MessageId,
        emoji: &str,
        present: bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if Instant::now() > deadline {
                panic!("timed out waiting for reaction state");
            }
            let message = tokio::time::timeout(Duration::from_secs(5), subscription.next_message())
                .await
                .expect("event in time")
                .expect("event decodes");
            let matches = match message.payload {
                ServerPayload::MessageAdded { message }
                | ServerPayload::MessageUpdated { message }
                    if message.id == *message_id =>
                {
                    message
                        .reactions
                        .iter()
                        .any(|reaction| reaction.emoji == emoji)
                }
                _ => continue,
            };
            if matches == present {
                return;
            }
        }
    }

    async fn wait_for_attachment(subscription: &mut handover_ipc::Subscription, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if Instant::now() > deadline {
                panic!("timed out waiting for attachment state");
            }
            let message = tokio::time::timeout(Duration::from_secs(5), subscription.next_message())
                .await
                .expect("event in time")
                .expect("event decodes");
            match message.payload {
                ServerPayload::MessageAdded { message }
                | ServerPayload::MessageUpdated { message }
                    if message
                        .attachments
                        .iter()
                        .any(|attachment| attachment.name.as_deref() == Some(name)) =>
                {
                    return;
                }
                _ => {}
            }
        }
    }

    async fn wait_for_message_removed(
        subscription: &mut handover_ipc::Subscription,
        message_id: &MessageId,
    ) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if Instant::now() > deadline {
                panic!("timed out waiting for message removal");
            }
            let message = tokio::time::timeout(Duration::from_secs(5), subscription.next_message())
                .await
                .expect("event in time")
                .expect("event decodes");
            if let ServerPayload::MessageRemoved {
                message_id: removed,
            } = message.payload
            {
                if removed == *message_id {
                    return;
                }
            }
        }
    }

    async fn wait_for_typing(
        subscription: &mut handover_ipc::Subscription,
        conversation: &ConversationId,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if Instant::now() > deadline {
                panic!("timed out waiting for typing");
            }
            let message = tokio::time::timeout(Duration::from_secs(5), subscription.next_message())
                .await
                .expect("event in time")
                .expect("event decodes");
            if let ServerPayload::Typing { state } = message.payload {
                if &state.conversation_id == conversation && !state.participant_ids.is_empty() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod messaging_failure_tests {
    use std::sync::{Arc, RwLock};

    use handover_core::{
        BatteryState, Capability, Conversation, ConversationEvent, ConversationId,
        ConversationKind, Device, DeviceId, Message, MessageId, MessagingAccount,
        MessagingAccountEvent, MessagingAccountId, MessagingEvent as CoreEvent, Participant,
        TransportKind,
    };
    use handover_ipc::{Client, ErrorCode, IpcError};
    use std::collections::BTreeSet;

    use super::*;

    fn seeded_state() -> Arc<RwLock<StateStore>> {
        let account_id = MessagingAccountId::new("gmessages:test");
        let conversation_id = ConversationId::new(account_id.clone(), "thread-1");
        let mut store = StateStore::default();
        store.apply(StateEvent::Messaging(CoreEvent::Account(
            MessagingAccountEvent::Added(MessagingAccount {
                id: account_id.clone(),
                label: "Test".into(),
                connected: true,
                authenticated: true,
            }),
        )));
        store.apply(StateEvent::Messaging(CoreEvent::Conversation(
            ConversationEvent::Added(Conversation {
                id: conversation_id.clone(),
                kind: ConversationKind::Direct,
                transport: TransportKind::Rcs,
                title: None,
                participants: vec![
                    Participant {
                        local_id: "self".into(),
                        display_name: None,
                        address: None,
                        is_self: true,
                    },
                    Participant {
                        local_id: "peer".into(),
                        display_name: None,
                        address: None,
                        is_self: false,
                    },
                ],
                latest_message_id: Some(MessageId::new(conversation_id.clone(), "m1")),
                last_activity_at: None,
                unread_count: None,
                cursor: None,
                capabilities: BTreeSet::from([handover_core::MessagingCapability::Text]),
            }),
        )));
        store.apply(StateEvent::Messaging(CoreEvent::Message(
            handover_core::MessageEvent::Added(Message {
                id: MessageId::new(conversation_id, "m1"),
                sender: Participant {
                    local_id: "peer".into(),
                    display_name: None,
                    address: None,
                    is_self: false,
                },
                transport: None,
                sent_at: Some(10),
                text: Some("hello".into()),
                attachments: vec![],
                reply_to: None,
                reactions: vec![],
                deleted: false,
            }),
        )));
        Arc::new(RwLock::new(store))
    }

    async fn server_without_helper(
        state: Arc<RwLock<StateStore>>,
    ) -> (tempfile::TempDir, PathBuf, tokio::task::JoinHandle<()>) {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("handoverd.sock");
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let server = IpcServer::bind_at(path.clone(), Arc::clone(&state), events, None)
            .await
            .expect("server binds");
        let task = tokio::spawn(async move {
            server.run().await.expect("server runs");
        });
        (directory, path, task)
    }

    #[tokio::test]
    async fn helper_down_reports_unavailable_without_touching_devices() {
        let state = seeded_state();
        state
            .write()
            .unwrap()
            .apply(StateEvent::Device(DeviceEvent::Added(Device {
                id: DeviceId::new("phone"),
                name: "Phone".into(),
                connected: true,
                paired: true,
                battery: Some(BatteryState::new(72, false).expect("valid")),
                connectivity: None,
                capabilities: BTreeSet::from([Capability::Battery]),
            })));
        let (_directory, path, task) = server_without_helper(state).await;
        let mut client = Client::connect_to(path).await.expect("connect");

        // Devices keep working with no helper configured.
        assert_eq!(client.devices().await.expect("devices").len(), 1);

        // Validation still runs first: unknown targets report as unknown.
        let unknown = client
            .send_message_text(
                ConversationId::new(MessagingAccountId::new("gmessages:test"), "missing"),
                "hi".into(),
            )
            .await
            .expect_err("unknown conversation");
        assert!(matches!(
            unknown,
            IpcError::Server {
                code: ErrorCode::UnknownConversation,
                ..
            }
        ));

        // Known targets fail closed with MessagingUnavailable, never with a
        // guessed acceptance.
        let down = client
            .send_message_text(
                ConversationId::new(MessagingAccountId::new("gmessages:test"), "thread-1"),
                "hi".into(),
            )
            .await
            .expect_err("helper down");
        assert!(matches!(
            down,
            IpcError::Server {
                code: ErrorCode::MessagingUnavailable,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn login_rejects_bad_bundles_before_helper_contact() {
        let (_directory, path, task) = server_without_helper(seeded_state()).await;
        let mut client = Client::connect_to(path).await.expect("connect");
        let error = client
            .messaging_login(MessagingAccountId::new("gmessages:test"), String::new())
            .await
            .expect_err("empty bundle");
        assert!(matches!(
            error,
            IpcError::Server {
                code: ErrorCode::CredentialRejected,
                ..
            }
        ));
        // An oversized bundle cannot even cross the 64 KiB IPC line limit:
        // it fails closed at the transport (line-too-long, a closed
        // connection, or a broken pipe on the racing write), never reaching
        // the helper.
        let error = client
            .messaging_login(
                MessagingAccountId::new("gmessages:test"),
                "x".repeat(256 * 1024 + 1),
            )
            .await
            .expect_err("oversized bundle");
        assert!(
            matches!(
                error,
                IpcError::LineTooLong
                    | IpcError::ConnectionClosed
                    | IpcError::Server { .. }
                    | IpcError::Io(_)
            ),
            "oversized bundle must fail closed, got {error:?}"
        );
        task.abort();
    }

    #[tokio::test]
    async fn history_gap_without_helper_is_unavailable() {
        let (_directory, path, task) = server_without_helper(seeded_state()).await;
        let mut client = Client::connect_to(path).await.expect("connect");
        // No helper is configured, so an aged-out cursor cannot be paged:
        // the subsystem reports itself unavailable rather than guessing.
        let error = client
            .messaging_history(
                ConversationId::new(MessagingAccountId::new("gmessages:test"), "thread-1"),
                None,
                Some("aged-out".into()),
            )
            .await
            .expect_err("cursor gap");
        assert!(matches!(
            error,
            IpcError::Server {
                code: ErrorCode::MessagingUnavailable,
                ..
            }
        ));
        task.abort();
    }

    #[tokio::test]
    async fn unknown_account_stays_unknown() {
        let (_directory, path, task) = server_without_helper(seeded_state()).await;
        let mut client = Client::connect_to(path).await.expect("connect");
        let error = client
            .messaging_conversations(MessagingAccountId::new("gmessages:ghost"))
            .await
            .expect_err("unknown account");
        assert!(matches!(
            error,
            IpcError::Server {
                code: ErrorCode::UnknownMessagingAccount,
                ..
            }
        ));
        task.abort();
    }
}
