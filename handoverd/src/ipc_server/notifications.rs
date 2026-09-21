use super::*;

pub(crate) async fn handle_notification_command<W>(
    command: NotificationCommand,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let validation = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .validate_command(&command);
    if let Err(error) = validation {
        let (code, message) = match error {
            CommandValidationError::NotFound => (
                ErrorCode::NotificationNotFound,
                "notification is no longer available",
            ),
            CommandValidationError::NotClearable => (
                ErrorCode::InvalidNotificationCommand,
                "notification is not clearable",
            ),
            CommandValidationError::UnknownAction => (
                ErrorCode::InvalidNotificationCommand,
                "notification does not provide that action",
            ),
            CommandValidationError::ReplyUnsupported => (
                ErrorCode::InvalidNotificationCommand,
                "notification does not support replies",
            ),
            CommandValidationError::EmptyReply => (
                ErrorCode::InvalidNotificationCommand,
                "reply text must not be empty",
            ),
        };
        write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
        return Ok(true);
    }

    let notification_id = command.notification_id().clone();
    if notification_id.device_id.as_str().starts_with("native:") {
        return handle_native_notification_command(command, notification_id, writer).await;
    }
    match handover_kdeconnect::KdeConnectBackend::execute(&command).await {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::CommandCompleted { notification_id }),
            )
            .await?;
        }
        Err(error) => {
            warn!(%error, %notification_id, "KDE Connect rejected notification command");
            write_json_line(
                writer,
                &ServerMessage::protocol_error(
                    ErrorCode::BackendRejected,
                    "notification backend could not complete the command",
                ),
            )
            .await?;
        }
    }
    Ok(true)
}

/// Route a validated notification command to the paired native session.
/// A successful response means the command was accepted for delivery to the
/// phone, not that Android confirmed the dismissal, action, or reply. State
/// changes arrive later as ordinary notification events.
pub(crate) async fn handle_native_notification_command<W>(
    command: NotificationCommand,
    notification_id: handover_core::NotificationId,
    writer: &mut W,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let Some(native) = native_backend() else {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(
                ErrorCode::BackendUnavailable,
                "native backend unavailable",
            ),
        )
        .await?;
        return Ok(true);
    };
    let peer_id = notification_id
        .device_id
        .as_str()
        .strip_prefix("native:")
        .unwrap_or_default();
    match native.execute_notification(peer_id, &command) {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::CommandCompleted { notification_id }),
            )
            .await?;
        }
        Err(error) => {
            use handover_native::NativeCommandError;
            let (code, message) = match error {
                NativeCommandError::Offline => (
                    ErrorCode::DeviceDisconnected,
                    "native device is disconnected",
                ),
                NativeCommandError::QueueFull => (
                    ErrorCode::BackendRejected,
                    "native backend could not accept the command",
                ),
            };
            warn!(%notification_id, "native notification command not accepted");
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
        }
    }
    Ok(true)
}
