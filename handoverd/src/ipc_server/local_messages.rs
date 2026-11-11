use super::*;

#[cfg(test)]
mod tests;

pub(super) async fn handle<W: AsyncWrite + Unpin>(
    method: Method,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
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
        Method::MessagesContext { message_id } => {
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
            } else if let Some(archive) = archive {
                match archive.context(message_id.clone()).await {
                    Ok(Some(messages)) => {
                        let id = message_id.conversation_id;
                        protocol::write_list_response(
                            writer,
                            messages,
                            |messages| ServerPayload::MessageContext {
                                conversation_id: id.clone(),
                                messages: messages.to_vec(),
                                done: true,
                            },
                            |messages, done| ServerPayload::MessageContext {
                                conversation_id: id.clone(),
                                messages: messages.to_vec(),
                                done,
                            },
                        )
                        .await?;
                        return Ok(true);
                    }
                    Ok(None) => error(ErrorCode::UnknownMessage, "Saved message was not found"),
                    Err(_) => error(
                        ErrorCode::HistoryUnavailable,
                        "Saved context is unavailable",
                    ),
                }
            } else {
                error(
                    ErrorCode::HistoryUnavailable,
                    "Local history storage is unavailable",
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
        _ => unreachable!("local messaging method"),
    };
    write_json_line(writer, &response).await?;
    Ok(true)
}
