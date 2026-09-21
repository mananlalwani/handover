use super::*;

pub(crate) fn valid_cancel_transfer_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) async fn handle_share_cancel<W>(
    device_id: DeviceId,
    transfer_id: String,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
    events: &broadcast::Sender<StateEvent>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let Some(peer_id) = device_id.as_str().strip_prefix("native:") else {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(
                ErrorCode::UnknownDevice,
                "only native shares can be cancelled",
            ),
        )
        .await?;
        return Ok(true);
    };
    if !valid_cancel_transfer_id(&transfer_id) {
        write_json_line(
            writer,
            &ServerMessage::protocol_error(ErrorCode::MalformedRequest, "unknown transfer"),
        )
        .await?;
        return Ok(true);
    }
    let cancelled =
        native_backend().is_some_and(|native| native.cancel_share(peer_id, &transfer_id));
    if cancelled {
        // The daemon reports the user cancellation; the receiver never sees
        // the transfer, so no receiver acknowledgement follows.
        apply_backend_event(
            state,
            events,
            StateEvent::ShareResult(handover_core::ShareResult {
                device_id,
                transfer_id,
                status: handover_core::ShareStatus::Failed,
                reason: Some(handover_core::ShareFailure::Rejected),
            }),
        );
    }
    write_json_line(
        writer,
        &ServerMessage::new(ServerPayload::ShareCancelled { cancelled }),
    )
    .await?;
    Ok(true)
}

pub(crate) async fn handle_share_command<W>(
    device_id: DeviceId,
    value: String,
    is_file: bool,
    writer: &mut W,
    state: &Arc<RwLock<StateStore>>,
) -> Result<bool, IpcError>
where
    W: AsyncWrite + Unpin,
{
    let target = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_device(&device_id)
        .cloned();
    let rejected = match target {
        None => Some((ErrorCode::UnknownDevice, "device is not known")),
        Some(device) if !device.connected => {
            Some((ErrorCode::DeviceDisconnected, "device is disconnected"))
        }
        Some(device) if !device.paired => {
            Some((ErrorCode::DeviceNotPaired, "device is not paired"))
        }
        Some(device) if !device.capabilities.contains(&Capability::FileTransfer) => Some((
            ErrorCode::UnsupportedCapability,
            "device does not provide file and URL sharing",
        )),
        Some(_) => None,
    };
    if let Some((code, message)) = rejected {
        write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
        return Ok(true);
    }

    let resource = if is_file {
        validate_file_url(&value).await
    } else {
        validate_outgoing_url(&value)
    };
    let url = match resource {
        Ok(url) => url,
        Err(code) => {
            let message = match code {
                ErrorCode::ResourceNotFound => "file does not exist",
                _ => "invalid or unreadable share resource",
            };
            write_json_line(writer, &ServerMessage::protocol_error(code, message)).await?;
            return Ok(true);
        }
    };

    if let Some(peer_id) = device_id.as_str().strip_prefix("native:") {
        let result = native_backend()
            .ok_or(handover_native::NativeCommandError::Offline)
            .and_then(|native| {
                if is_file {
                    let path = Url::parse(&url)
                        .ok()
                        .and_then(|url| url.to_file_path().ok())
                        .ok_or(handover_native::NativeCommandError::QueueFull)?;
                    // Copy into private staging before queueing: the
                    // backend opens the path later, and the source may
                    // live in a directory writable by another local user.
                    let staged = crate::send_staging::stage_send_file(&path)
                        .map_err(|_| handover_native::NativeCommandError::QueueFull)?;
                    match native.share_file(peer_id, staged.clone()) {
                        Ok(transfer_id) => {
                            crate::send_staging::note_accepted(&transfer_id, &staged);
                            Ok(transfer_id)
                        }
                        Err(error) => {
                            // Never queued: remove the staged copy now.
                            if let Some(directory) = staged.parent() {
                                let _ = std::fs::remove_dir_all(directory);
                            }
                            Err(error)
                        }
                    }
                } else {
                    native.share_url(peer_id, url)
                }
            });
        let payload = match result {
            Ok(transfer_id) => ServerMessage::new(ServerPayload::ShareAccepted {
                device_id,
                transfer_id: Some(transfer_id),
            }),
            Err(handover_native::NativeCommandError::Offline) => ServerMessage::protocol_error(
                ErrorCode::DeviceDisconnected,
                "native device is disconnected",
            ),
            Err(handover_native::NativeCommandError::QueueFull) => ServerMessage::protocol_error(
                ErrorCode::BackendRejected,
                "native backend could not accept the share",
            ),
        };
        write_json_line(writer, &payload).await?;
        return Ok(true);
    }

    match handover_kdeconnect::KdeConnectBackend::share_url(&device_id, &url).await {
        Ok(()) => {
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::ShareAccepted {
                    device_id,
                    transfer_id: None,
                }),
            )
            .await?;
        }
        Err(error) => {
            let unavailable =
                matches!(error, handover_kdeconnect::CommandError::BackendUnavailable);
            warn!(%device_id, is_file, unavailable, "KDE Connect rejected share request");
            write_json_line(
                writer,
                &ServerMessage::protocol_error(
                    if unavailable {
                        ErrorCode::BackendUnavailable
                    } else {
                        ErrorCode::BackendRejected
                    },
                    if unavailable {
                        "sharing backend is unavailable"
                    } else {
                        "sharing backend could not accept the request"
                    },
                ),
            )
            .await?;
        }
    }
    Ok(true)
}

pub(crate) fn validate_outgoing_url(input: &str) -> Result<String, ErrorCode> {
    if input.is_empty() || input.trim() != input || input.chars().any(char::is_control) {
        return Err(ErrorCode::InvalidResource);
    }
    let url = Url::parse(input).map_err(|_| ErrorCode::InvalidResource)?;
    if matches!(url.scheme(), "file" | "javascript" | "data") {
        return Err(ErrorCode::InvalidResource);
    }
    Ok(url.into())
}

pub(crate) async fn validate_file_url(input: &str) -> Result<String, ErrorCode> {
    let url = Url::parse(input).map_err(|_| ErrorCode::InvalidResource)?;
    if url.scheme() != "file" || url.query().is_some() || url.fragment().is_some() {
        return Err(ErrorCode::InvalidResource);
    }
    let path = url.to_file_path().map_err(|_| ErrorCode::InvalidResource)?;
    let metadata = tokio::fs::metadata(&path).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ErrorCode::ResourceNotFound
        } else {
            ErrorCode::InvalidResource
        }
    })?;
    if !metadata.is_file() {
        return Err(ErrorCode::InvalidResource);
    }
    tokio::fs::File::open(&path)
        .await
        .map_err(|_| ErrorCode::InvalidResource)?;
    Ok(url.into())
}
