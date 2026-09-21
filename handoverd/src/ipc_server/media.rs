use super::*;

pub(crate) async fn handle_media_command<W>(
    command: MediaCommand,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let validation = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .validate_media_command(&command);
    if let Err(error) = validation {
        let (code, message) = match error {
            MediaValidationError::NotFound => (
                ErrorCode::MediaSessionNotFound,
                "media session is no longer available",
            ),
            MediaValidationError::DeviceUnavailable => (
                ErrorCode::DeviceDisconnected,
                "source device is unavailable",
            ),
            MediaValidationError::UnsupportedControl => (
                ErrorCode::InvalidMediaCommand,
                "media session does not support that control",
            ),
            MediaValidationError::InvalidValue => (
                ErrorCode::InvalidMediaCommand,
                "invalid media control value",
            ),
        };
        write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
        return Ok(true);
    }

    let id = command.id().clone();
    if id.device_id.as_str().starts_with("native:") {
        return handle_native_media_command(command, id, writer).await;
    }
    match handover_kdeconnect::KdeConnectBackend::execute_media(&command).await {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::MediaAccepted { id }),
            )
            .await?
        }
        Err(error) => {
            warn!(device_id = %id.device_id, player_id = %id.player_id, %error, "KDE Connect rejected media command");
            write_json_line(
                writer,
                &ServerMessage::protocol_error(
                    ErrorCode::BackendRejected,
                    "media backend could not accept the command",
                ),
            )
            .await?;
        }
    }
    Ok(true)
}

/// Route a validated media command to the paired native session.
/// A successful response means the command was accepted for delivery to the
/// phone, not that Android changed playback. Authoritative state arrives as
/// a later media update.
pub(crate) async fn handle_native_media_command<W>(
    command: MediaCommand,
    id: handover_core::MediaSessionId,
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
    let peer_id = id
        .device_id
        .as_str()
        .strip_prefix("native:")
        .unwrap_or_default();
    match native.execute_media(peer_id, &command) {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::MediaAccepted { id }),
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
            warn!(device_id = %id.device_id, player_id = %id.player_id, "native media command not accepted");
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
        }
    }
    Ok(true)
}
