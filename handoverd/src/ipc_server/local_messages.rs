use super::*;

#[cfg(test)]
mod tests;

pub(super) async fn handle<W: AsyncWrite + Unpin>(
    method: Method,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    messaging: &Option<crate::messaging_backend::MessagingHub>,
) -> Result<bool, IpcError> {
    let (workspace, archive, accounts) = {
        let state = state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            state.message_workspace.clone(),
            state.message_archive.clone(),
            state
                .messaging()
                .snapshot_accounts()
                .into_iter()
                .map(|account| account.id)
                .collect::<std::collections::BTreeSet<_>>(),
        )
    };
    let known = |id: &handover_core::ConversationId| {
        accounts.contains(&id.account_id) && handover_core::validate_conversation_id(id).is_ok()
    };
    let error = |code, message: &str| ServerMessage::protocol_error(code, message);
    let response = match method {
        Method::MessagesSearch {
            query,
            account_id,
            offset,
        } => {
            if query.chars().count() > 256
                || query.split_whitespace().count() > 16
                || offset > 10_000
            {
                error(
                    ErrorCode::InvalidMessagingCommand,
                    "Search is limited to 256 characters, 16 terms, and 10000 results",
                )
            } else if account_id.as_ref().is_some_and(|id| !accounts.contains(id)) {
                error(ErrorCode::UnknownMessagingAccount, "Account is unavailable")
            } else if let Some(archive) = archive {
                match archive
                    .search(query, account_id.map(|id| id.as_str().to_owned()), offset)
                    .await
                {
                    Ok((mut hits, has_more)) => {
                        hits.retain(|hit| known(&hit.message_id.conversation_id));
                        ServerMessage::new(ServerPayload::MessageSearch { hits, has_more })
                    }
                    Err(_) => error(
                        ErrorCode::HistoryUnavailable,
                        "Saved-message search is unavailable",
                    ),
                }
            } else {
                error(
                    ErrorCode::HistoryUnavailable,
                    "Local history storage is unavailable",
                )
            }
        }
        Method::MessagesSearchConversation {
            conversation_id,
            query,
            offset,
        } => {
            if handover_core::validate_conversation_id(&conversation_id).is_err()
                || query.chars().count() > 256
                || query.split_whitespace().count() > 16
                || offset > 10_000
            {
                error(
                    ErrorCode::InvalidMessagingCommand,
                    "Conversation search requires a valid conversation and search within the search bounds",
                )
            } else if !known(&conversation_id) {
                error(ErrorCode::UnknownMessagingAccount, "Account is unavailable")
            } else if let Some(archive) = archive {
                match archive
                    .search_conversation(query, conversation_id, offset)
                    .await
                {
                    Ok((hits, has_more)) => {
                        ServerMessage::new(ServerPayload::MessageSearch { hits, has_more })
                    }
                    Err(_) => error(
                        ErrorCode::HistoryUnavailable,
                        "Saved-message search is unavailable",
                    ),
                }
            } else {
                error(
                    ErrorCode::HistoryUnavailable,
                    "Local history storage is unavailable",
                )
            }
        }
        Method::MessagesMedia {
            conversation_id,
            offset,
        } => {
            if handover_core::validate_conversation_id(&conversation_id).is_err() || offset > 10_000
            {
                error(
                    ErrorCode::InvalidMessagingCommand,
                    "Media listing requires a valid conversation and offset at most 10000",
                )
            } else if !known(&conversation_id) {
                error(ErrorCode::UnknownMessagingAccount, "Account is unavailable")
            } else if let Some(archive) = archive {
                match archive.media(conversation_id.clone(), offset).await {
                    Ok((messages, has_more)) => {
                        protocol::write_list_response(
                            writer,
                            messages,
                            |messages| ServerPayload::MessageMedia {
                                conversation_id: conversation_id.clone(),
                                messages: messages.to_vec(),
                                has_more,
                                done: true,
                            },
                            |messages, done| ServerPayload::MessageMedia {
                                conversation_id: conversation_id.clone(),
                                messages: messages.to_vec(),
                                has_more,
                                done,
                            },
                        )
                        .await?;
                        return Ok(true);
                    }
                    Err(_) => error(
                        ErrorCode::HistoryUnavailable,
                        "Saved media listing is unavailable",
                    ),
                }
            } else {
                error(
                    ErrorCode::HistoryUnavailable,
                    "Local history storage is unavailable",
                )
            }
        }
        Method::MessagesPreviews { conversation_ids } => {
            if conversation_ids.len() > 50
                || conversation_ids
                    .iter()
                    .any(|id| handover_core::validate_conversation_id(id).is_err())
            {
                error(
                    ErrorCode::InvalidMessagingCommand,
                    "Previews require at most 50 valid conversation identities",
                )
            } else if conversation_ids.iter().any(|id| !known(id)) {
                error(ErrorCode::UnknownMessagingAccount, "Account is unavailable")
            } else if let Some(archive) = archive {
                let ids = conversation_ids
                    .into_iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                match archive.previews(ids).await {
                    Ok(messages) => {
                        protocol::write_list_response(
                            writer,
                            messages,
                            |messages| ServerPayload::MessagePreviews {
                                messages: messages.to_vec(),
                                done: true,
                            },
                            |messages, done| ServerPayload::MessagePreviews {
                                messages: messages.to_vec(),
                                done,
                            },
                        )
                        .await?;
                        return Ok(true);
                    }
                    Err(_) => error(
                        ErrorCode::HistoryUnavailable,
                        "Saved message previews are unavailable",
                    ),
                }
            } else {
                error(
                    ErrorCode::HistoryUnavailable,
                    "Local history storage is unavailable",
                )
            }
        }
        Method::MessagesContext {
            message_id,
            refresh,
        } => {
            if handover_core::validate_messaging_location(&handover_core::MessagingLocation {
                conversation_id: message_id.conversation_id.clone(),
                message_id: Some(message_id.clone()),
            })
            .is_err()
            {
                error(
                    ErrorCode::InvalidMessagingCommand,
                    "Invalid message identity",
                )
            } else if !known(&message_id.conversation_id) {
                error(ErrorCode::UnknownMessagingAccount, "Account is unavailable")
            } else {
                let saved = if !refresh && let Some(archive) = &archive {
                    archive
                        .context_page(message_id.clone())
                        .await
                        .map_err(|_| ())
                } else {
                    Err(())
                };
                let mut unavailable = saved.is_err();
                let mut page = saved.ok().flatten();
                if page.is_none() {
                    let online = state
                        .read()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .messaging()
                        .account(&message_id.conversation_id.account_id)
                        .is_some_and(|account| account.connected && account.authenticated);
                    if online && let Some(messaging) = messaging {
                        let source_page = if refresh && let Some(archive) = &archive {
                            archive.page_cursor(message_id.clone()).await.ok().flatten()
                        } else {
                            None
                        };
                        let result = if let Some(before) = source_page {
                            let fetched = messaging
                                .fetch_through_helper(
                                    message_id.conversation_id.account_id.as_str(),
                                    &message_id.conversation_id.local_id,
                                    100,
                                    before.clone(),
                                )
                                .await;
                            match (fetched, &archive) {
                                (Ok(()), Some(archive)) => {
                                    match archive
                                        .history(
                                            message_id.conversation_id.clone(),
                                            100,
                                            before,
                                            None,
                                        )
                                        .await
                                    {
                                        Ok(Some((records, _))) => Ok(records
                                            .into_iter()
                                            .find(|message| message.id == message_id)),
                                        _ => Err(
                                            crate::messaging_backend::HelperCallError::Unavailable,
                                        ),
                                    }
                                }
                                _ => Err(crate::messaging_backend::HelperCallError::Unavailable),
                            }
                        } else {
                            messaging.fetch_message(message_id.clone()).await
                        };
                        match result {
                            Ok(message) => {
                                page = message.map(|message| (vec![message], None));
                                unavailable = false;
                            }
                            Err(_) => unavailable = true,
                        }
                    }
                }
                if let Some((messages, cursor)) = page {
                    let id = message_id.conversation_id;
                    protocol::write_list_response(
                        writer,
                        messages,
                        |messages| ServerPayload::MessageContext {
                            conversation_id: id.clone(),
                            messages: messages.to_vec(),
                            done: true,
                            cursor_next: cursor.clone(),
                        },
                        |messages, done| ServerPayload::MessageContext {
                            conversation_id: id.clone(),
                            messages: messages.to_vec(),
                            done,
                            cursor_next: cursor.clone(),
                        },
                    )
                    .await?;
                    return Ok(true);
                }
                error(
                    if unavailable {
                        ErrorCode::HistoryUnavailable
                    } else {
                        ErrorCode::UnknownMessage
                    },
                    "Original message is unavailable",
                )
            }
        }

        Method::MessagesWorkspace | Method::MessagesSelect { .. } => {
            let result = match method {
                Method::MessagesSelect { location } => {
                    if location.as_ref().is_some_and(|location| {
                        handover_core::validate_messaging_location(location).is_err()
                    }) {
                        write_json_line(
                            writer,
                            &error(ErrorCode::InvalidMessagingCommand, "Invalid saved location"),
                        )
                        .await?;
                        return Ok(true);
                    }
                    if location
                        .as_ref()
                        .is_some_and(|location| !known(&location.conversation_id))
                    {
                        write_json_line(
                            writer,
                            &error(ErrorCode::UnknownMessagingAccount, "Account is unavailable"),
                        )
                        .await?;
                        return Ok(true);
                    }
                    workspace.select(location).await
                }
                _ => workspace.get().await,
            };
            match result {
                Ok(mut workspace) => {
                    workspace.draft_conversations.retain(&known);
                    if workspace
                        .location
                        .as_ref()
                        .is_some_and(|location| !known(&location.conversation_id))
                    {
                        workspace.location = None;
                    }
                    ServerMessage::new(ServerPayload::MessagingWorkspace { workspace })
                }
                Err(_) => error(
                    ErrorCode::BackendUnavailable,
                    "Messaging workspace is unavailable",
                ),
            }
        }
        Method::MessagesDraft { conversation_id } => {
            if !known(&conversation_id) {
                error(ErrorCode::UnknownMessagingAccount, "Account is unavailable")
            } else {
                match workspace.draft(conversation_id).await {
                    Ok(draft) => ServerMessage::new(ServerPayload::MessageDraft { draft }),
                    Err(_) => error(ErrorCode::BackendUnavailable, "Draft is unavailable"),
                }
            }
        }
        Method::MessagesSaveDraft { draft } => {
            if !known(&draft.conversation_id) {
                error(ErrorCode::UnknownMessagingAccount, "Account is unavailable")
            } else {
                match workspace.save(draft).await {
                    Ok(draft) => ServerMessage::new(ServerPayload::MessageDraft { draft }),
                    Err(crate::messaging_workspace::WorkspaceError::Conflict) => error(
                        ErrorCode::DraftConflict,
                        "Draft changed in another window; reload it before replacing it",
                    ),
                    Err(crate::messaging_workspace::WorkspaceError::Invalid(_)) => error(
                        ErrorCode::InvalidMessagingCommand,
                        "Draft exceeds its limits or has invalid scope",
                    ),
                    Err(_) => error(ErrorCode::BackendUnavailable, "Draft could not be saved"),
                }
            }
        }
        Method::MessagesResolveDraftSend { draft } => {
            use handover_core::{MessageStatus, OutgoingOperationKind, OutgoingOutcome};
            let operation = draft.pending_operation.as_ref().and_then(|id| {
                state
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .messaging()
                    .outgoing(id)
                    .cloned()
            });
            let valid = known(&draft.conversation_id)
                && draft.pending_operation.is_some()
                && operation.as_ref().is_none_or(|operation| {
                    operation.conversation_id == draft.conversation_id
                        && operation.kind == OutgoingOperationKind::Text
                });
            let resolution = match operation.as_ref().map(|operation| &operation.outcome) {
                Some(OutgoingOutcome::Provider(
                    MessageStatus::Sent | MessageStatus::Delivered | MessageStatus::Displayed,
                )) => Some(true),
                None
                | Some(
                    OutgoingOutcome::Unknown
                    | OutgoingOutcome::Rejected
                    | OutgoingOutcome::Provider(MessageStatus::Failed(_)),
                ) => Some(false),
                _ => None,
            };
            if let Some(sent) = resolution.filter(|_| valid) {
                let operation = draft
                    .pending_operation
                    .clone()
                    .expect("validated pending operation");
                match workspace.draft_send(draft, operation, Some(sent)).await {
                    Ok(draft) => ServerMessage::new(ServerPayload::MessageDraft { draft }),
                    Err(crate::messaging_workspace::WorkspaceError::Conflict) => error(
                        ErrorCode::DraftConflict,
                        "Draft changed; reload it before continuing",
                    ),
                    Err(_) => error(
                        ErrorCode::BackendUnavailable,
                        "Unable to save send recovery",
                    ),
                }
            } else {
                error(
                    ErrorCode::InvalidMessagingCommand,
                    "Send is still in progress",
                )
            }
        }
        Method::MessagesAttachmentDraft { conversation_id } => {
            if !known(&conversation_id) {
                error(ErrorCode::UnknownMessagingAccount, "Account is unavailable")
            } else {
                match workspace.attachment_draft(conversation_id).await {
                    Ok(draft) => {
                        ServerMessage::new(ServerPayload::MessageAttachmentDraft { draft })
                    }
                    Err(_) => error(
                        ErrorCode::BackendUnavailable,
                        "Attachment draft is unavailable",
                    ),
                }
            }
        }
        Method::MessagesAttachmentSend {
            conversation_id,
            operation_id,
            caption,
            revision,
            resolve,
        } => {
            use handover_core::{MessageStatus, OutgoingOperationKind, OutgoingOutcome};
            let operation = state
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .messaging()
                .outgoing(&operation_id)
                .cloned();
            let valid = known(&conversation_id)
                && operation.as_ref().is_none_or(|operation| {
                    operation.conversation_id == conversation_id
                        && operation.kind == OutgoingOperationKind::Media
                });
            let resolution = if resolve {
                match operation.as_ref().map(|operation| &operation.outcome) {
                    Some(OutgoingOutcome::Provider(
                        MessageStatus::Sent | MessageStatus::Delivered | MessageStatus::Displayed,
                    )) => Some(Some(true)),
                    None
                    | Some(
                        OutgoingOutcome::Unknown
                        | OutgoingOutcome::Rejected
                        | OutgoingOutcome::Provider(MessageStatus::Failed(_)),
                    ) => Some(Some(false)),
                    _ => None,
                }
            } else if operation.is_some() {
                Some(None)
            } else {
                None
            };
            if let Some(resolution) = resolution.filter(|_| valid) {
                match workspace
                    .attachment_send(conversation_id, operation_id, caption, revision, resolution)
                    .await
                {
                    Ok(draft) => {
                        ServerMessage::new(ServerPayload::MessageAttachmentDraft { draft })
                    }
                    Err(crate::messaging_workspace::WorkspaceError::Conflict) => error(
                        ErrorCode::DraftConflict,
                        "Attachment draft changed; reload it before continuing",
                    ),
                    Err(_) => error(
                        ErrorCode::BackendUnavailable,
                        "Unable to save attachment send state",
                    ),
                }
            } else {
                error(
                    ErrorCode::InvalidMessagingCommand,
                    "Attachment send is not ready to resolve",
                )
            }
        }
        Method::MessagesSaveAttachmentDraft {
            queued_files,
            conversation_id,
            file_url,
            mime,
            revision,
        } => {
            if !known(&conversation_id) {
                error(ErrorCode::UnknownMessagingAccount, "Account is unavailable")
            } else {
                match workspace
                    .save_attachment_queue(conversation_id, file_url, mime, queued_files, revision)
                    .await
                {
                    Ok(draft) => {
                        let present = state
                            .read()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .messaging()
                            .snapshot_accounts()
                            .iter()
                            .any(|account| account.id == draft.conversation_id.account_id);
                        if present {
                            ServerMessage::new(ServerPayload::MessageAttachmentDraft { draft })
                        } else {
                            let _ = workspace
                                .remove_account(
                                    draft.conversation_id.account_id.as_str().to_owned(),
                                )
                                .await;
                            error(ErrorCode::UnknownMessagingAccount, "Account is unavailable")
                        }
                    }
                    Err(crate::messaging_workspace::WorkspaceError::Conflict) => error(
                        ErrorCode::DraftConflict,
                        "Attachment draft changed in another window; reload it before replacing it",
                    ),
                    Err(crate::messaging_workspace::WorkspaceError::Invalid(_)) => error(
                        ErrorCode::InvalidMessagingCommand,
                        "Attachment draft is invalid",
                    ),
                    Err(_) => error(
                        ErrorCode::BackendUnavailable,
                        "Attachment draft could not be saved",
                    ),
                }
            }
        }
        _ => unreachable!("local messaging method"),
    };
    write_json_line(writer, &response).await?;
    Ok(true)
}
