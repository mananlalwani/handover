use super::*;

pub(crate) async fn remove_stale_socket(path: &Path) -> Result<(), ServerError> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_socket() {
        return Err(ServerError::UnsafeSocketPath(path.to_owned()));
    }
    if UnixStream::connect(path).await.is_ok() {
        return Err(ServerError::AlreadyRunning(path.to_owned()));
    }
    tokio::fs::remove_file(path).await?;
    Ok(())
}

pub(crate) async fn handle_client(
    stream: UnixStream,
    state: Arc<RwLock<StateStore>>,
    events: broadcast::Sender<StateEvent>,
    messaging: Option<MessagingHub>,
) -> Result<(), IpcError> {
    let (read_half, mut writer) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut subscription: Option<broadcast::Receiver<StateEvent>> = None;
    let mut flags = SubscriptionFlags::default();

    loop {
        if let Some(receiver) = subscription.as_mut() {
            tokio::select! {
                request = read_json_line::<_, Request>(&mut reader) => {
                    if !handle_request_result(request, &mut writer, &state, &events, &messaging, &mut subscription, &mut flags).await? {
                        return Ok(());
                    }
                }
                event = receiver.recv() => {
                    match event {
                        Ok(event) if matches!(event, StateEvent::ShareReceived(_) | StateEvent::ShareProgress(_) | StateEvent::ShareResult(_)) && !flags.shares => {}
                        Ok(event) if matches!(event, StateEvent::Media(_)) && !flags.media => {}
                        Ok(event) if matches!(event, StateEvent::Messaging(_)) && !flags.messages => {}
                        Ok(event) => write_json_line(&mut writer, &message_from_event(event)).await?,
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            debug!(skipped, "IPC client lagged; sending current snapshot");
                            write_snapshot_response(
                                &mut writer,
                                snapshot_payload(&state, flags.media, flags.messages),
                            )
                            .await?;
                        }
                        Err(broadcast::error::RecvError::Closed) => return Ok(()),
                    }
                }
            }
        } else {
            let Some(request) =
                read_pre_subscription_request(&mut reader, PRE_SUBSCRIPTION_TIMEOUT).await
            else {
                return Ok(());
            };
            if !handle_request_result(
                request,
                &mut writer,
                &state,
                &events,
                &messaging,
                &mut subscription,
                &mut flags,
            )
            .await?
            {
                return Ok(());
            }
        }
    }
}

pub(crate) async fn read_pre_subscription_request<R>(
    reader: &mut R,
    wait: Duration,
) -> Option<Result<Option<Request>, IpcError>>
where
    R: AsyncBufRead + Unpin,
{
    timeout(wait, read_json_line::<_, Request>(reader))
        .await
        .ok()
}

pub(crate) async fn write_conversation_chunks<W>(
    writer: &mut W,
    conversations: Vec<handover_core::Conversation>,
) -> Result<(), IpcError>
where
    W: AsyncWrite + Unpin,
{
    let mut chunk = Vec::new();
    for conversation in conversations {
        chunk.push(conversation);
        let candidate = ServerMessage::new(ServerPayload::ConversationsChunk {
            conversations: chunk.clone(),
            done: false,
        });
        if serde_json::to_vec(&candidate)
            .is_ok_and(|encoded| encoded.len() + 1 > handover_ipc::MAX_LINE_BYTES)
        {
            let last = chunk.pop().expect("chunk contains the candidate");
            if chunk.is_empty() {
                return Err(IpcError::LineTooLong);
            }
            write_json_line(
                writer,
                &ServerMessage::new(ServerPayload::ConversationsChunk {
                    conversations: chunk,
                    done: false,
                }),
            )
            .await?;
            chunk = vec![last];
        }
    }
    write_json_line(
        writer,
        &ServerMessage::new(ServerPayload::ConversationsChunk {
            conversations: chunk,
            done: true,
        }),
    )
    .await
}

pub(crate) async fn write_snapshot_response<W>(
    writer: &mut W,
    payload: ServerPayload,
) -> Result<(), IpcError>
where
    W: AsyncWrite + Unpin,
{
    let ServerPayload::Snapshot {
        devices,
        notifications,
        media_sessions,
        calls,
        messaging_accounts,
        conversations,
        typing_states,
        read_states,
    } = payload
    else {
        return Err(IpcError::UnexpectedResponse("not a snapshot".into()));
    };
    let full = ServerMessage::new(ServerPayload::Snapshot {
        devices: devices.clone(),
        notifications: notifications.clone(),
        media_sessions: media_sessions.clone(),
        calls: calls.clone(),
        messaging_accounts: messaging_accounts.clone(),
        conversations: conversations.clone(),
        typing_states: typing_states.clone(),
        read_states: read_states.clone(),
    });
    if serde_json::to_vec(&full).map_or(true, |encoded| {
        encoded.len() + 1 > handover_ipc::MAX_LINE_BYTES
    }) {
        write_snapshot_chunks(
            writer,
            devices,
            notifications,
            media_sessions,
            calls,
            messaging_accounts,
            conversations,
            typing_states,
            read_states,
        )
        .await
    } else {
        write_json_line(writer, &full).await
    }
}

#[allow(clippy::too_many_arguments)]
#[derive(Clone, Copy)]
pub(crate) enum SnapshotChunkKind {
    Snapshot,
    Subscribed,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn write_snapshot_chunks<W>(
    writer: &mut W,
    devices: Vec<handover_core::Device>,
    notifications: Vec<handover_core::Notification>,
    media_sessions: Vec<handover_core::MediaSession>,
    calls: Vec<handover_core::CallState>,
    messaging_accounts: Vec<handover_core::MessagingAccount>,
    conversations: Vec<handover_core::Conversation>,
    typing_states: Vec<handover_core::TypingState>,
    read_states: Vec<handover_core::ReadState>,
) -> Result<(), IpcError>
where
    W: AsyncWrite + Unpin,
{
    write_state_chunks(
        writer,
        SnapshotChunkKind::Snapshot,
        devices,
        notifications,
        media_sessions,
        calls,
        messaging_accounts,
        conversations,
        typing_states,
        read_states,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn write_state_chunks<W>(
    writer: &mut W,
    kind: SnapshotChunkKind,
    devices: Vec<handover_core::Device>,
    notifications: Vec<handover_core::Notification>,
    media_sessions: Vec<handover_core::MediaSession>,
    calls: Vec<handover_core::CallState>,
    messaging_accounts: Vec<handover_core::MessagingAccount>,
    conversations: Vec<handover_core::Conversation>,
    typing_states: Vec<handover_core::TypingState>,
    read_states: Vec<handover_core::ReadState>,
) -> Result<(), IpcError>
where
    W: AsyncWrite + Unpin,
{
    let mut chunk = SnapshotChunkData {
        kind,
        ..Default::default()
    };
    for item in devices {
        if !chunk.push_device(item.clone()) {
            flush_snapshot_chunk(writer, &mut chunk).await?;
            if !chunk.push_device(item) {
                return Err(IpcError::LineTooLong);
            }
        }
    }
    for item in notifications {
        if !chunk.push_notification(item.clone()) {
            flush_snapshot_chunk(writer, &mut chunk).await?;
            if !chunk.push_notification(item) {
                return Err(IpcError::LineTooLong);
            }
        }
    }
    for item in media_sessions {
        if !chunk.push_media_session(item.clone()) {
            flush_snapshot_chunk(writer, &mut chunk).await?;
            if !chunk.push_media_session(item) {
                return Err(IpcError::LineTooLong);
            }
        }
    }
    for item in calls {
        if !chunk.push_call(item.clone()) {
            flush_snapshot_chunk(writer, &mut chunk).await?;
            if !chunk.push_call(item) {
                return Err(IpcError::LineTooLong);
            }
        }
    }
    for item in messaging_accounts {
        if !chunk.push_account(item.clone()) {
            flush_snapshot_chunk(writer, &mut chunk).await?;
            if !chunk.push_account(item) {
                return Err(IpcError::LineTooLong);
            }
        }
    }
    for item in conversations {
        if !chunk.push_conversation(item.clone()) {
            flush_snapshot_chunk(writer, &mut chunk).await?;
            if !chunk.push_conversation(item) {
                return Err(IpcError::LineTooLong);
            }
        }
    }
    for item in typing_states {
        if !chunk.push_typing(item.clone()) {
            flush_snapshot_chunk(writer, &mut chunk).await?;
            if !chunk.push_typing(item) {
                return Err(IpcError::LineTooLong);
            }
        }
    }
    for item in read_states {
        if !chunk.push_read(item.clone()) {
            flush_snapshot_chunk(writer, &mut chunk).await?;
            if !chunk.push_read(item) {
                return Err(IpcError::LineTooLong);
            }
        }
    }
    write_json_line(writer, &chunk.message(true)).await
}

pub(crate) struct SnapshotChunkData {
    kind: SnapshotChunkKind,
    devices: Vec<handover_core::Device>,
    notifications: Vec<handover_core::Notification>,
    media_sessions: Vec<handover_core::MediaSession>,
    calls: Vec<handover_core::CallState>,
    messaging_accounts: Vec<handover_core::MessagingAccount>,
    conversations: Vec<handover_core::Conversation>,
    typing_states: Vec<handover_core::TypingState>,
    read_states: Vec<handover_core::ReadState>,
}

impl Default for SnapshotChunkData {
    fn default() -> Self {
        Self {
            kind: SnapshotChunkKind::Snapshot,
            devices: Vec::new(),
            notifications: Vec::new(),
            media_sessions: Vec::new(),
            calls: Vec::new(),
            messaging_accounts: Vec::new(),
            conversations: Vec::new(),
            typing_states: Vec::new(),
            read_states: Vec::new(),
        }
    }
}

impl SnapshotChunkData {
    fn message(&self, done: bool) -> ServerMessage {
        let payload = match self.kind {
            SnapshotChunkKind::Snapshot => ServerPayload::SnapshotChunk {
                devices: self.devices.clone(),
                notifications: self.notifications.clone(),
                media_sessions: self.media_sessions.clone(),
                calls: self.calls.clone(),
                messaging_accounts: self.messaging_accounts.clone(),
                conversations: self.conversations.clone(),
                typing_states: self.typing_states.clone(),
                read_states: self.read_states.clone(),
                done,
            },
            SnapshotChunkKind::Subscribed => ServerPayload::SubscribedChunk {
                devices: self.devices.clone(),
                notifications: self.notifications.clone(),
                media_sessions: self.media_sessions.clone(),
                calls: self.calls.clone(),
                messaging_accounts: self.messaging_accounts.clone(),
                conversations: self.conversations.clone(),
                typing_states: self.typing_states.clone(),
                read_states: self.read_states.clone(),
                done,
            },
        };
        ServerMessage::new(payload)
    }
    fn fits(&self) -> bool {
        serde_json::to_vec(&self.message(false))
            .is_ok_and(|v| v.len() < handover_ipc::MAX_LINE_BYTES)
    }
}

macro_rules! snapshot_push {
    ($name:ident, $field:ident, $ty:ty) => {
        fn $name(&mut self, item: $ty) -> bool {
            self.$field.push(item);
            if self.fits() {
                true
            } else {
                self.$field.pop();
                false
            }
        }
    };
}
impl SnapshotChunkData {
    snapshot_push!(push_device, devices, handover_core::Device);
    snapshot_push!(
        push_notification,
        notifications,
        handover_core::Notification
    );
    snapshot_push!(
        push_media_session,
        media_sessions,
        handover_core::MediaSession
    );
    snapshot_push!(push_call, calls, handover_core::CallState);
    snapshot_push!(
        push_account,
        messaging_accounts,
        handover_core::MessagingAccount
    );
    snapshot_push!(
        push_conversation,
        conversations,
        handover_core::Conversation
    );
    snapshot_push!(push_typing, typing_states, handover_core::TypingState);
    snapshot_push!(push_read, read_states, handover_core::ReadState);
}

pub(crate) async fn flush_snapshot_chunk<W>(
    writer: &mut W,
    chunk: &mut SnapshotChunkData,
) -> Result<(), IpcError>
where
    W: AsyncWrite + Unpin,
{
    write_json_line(writer, &chunk.message(false)).await?;
    let kind = chunk.kind;
    *chunk = SnapshotChunkData {
        kind,
        ..Default::default()
    };
    Ok(())
}

pub(crate) enum WaylandClipboard {
    Text {
        text: String,
        html: Option<String>,
        uri: Option<String>,
    },
    File {
        path: PathBuf,
        mime: String,
    },
}

pub(crate) async fn read_wayland_clipboard() -> Result<WaylandClipboard, ()> {
    let offered = crate::clipboard::offered_types().await.ok_or(())?;
    let selected = offered.iter().find(|mime| mime.starts_with("image/"));
    if let Some(mime) = selected {
        let extension = mime
            .strip_prefix("image/")
            .unwrap_or("bin")
            .replace('/', "_");
        // Clipboard temps live in the runtime directory (0700), not the
        // world-writable temp dir: another local user must not be able
        // to swap the file between capture and streaming.
        let directory = runtime_directory().map_err(|_| ())?;
        let path = directory.join(format!(
            "handover-clipboard-{}-{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| ())?
                .as_nanos(),
            extension
        ));
        let mime_owned = mime.to_owned();
        let bytes = tokio::task::spawn_blocking(move || {
            let mut command = std::process::Command::new("wl-paste");
            command.args(["--no-newline", "--type", &mime_owned]);
            let output =
                crate::local_cmd::output_bounded(&mut command, 10 * 1024 * 1024).map_err(|_| ())?;
            if !output.status.success() || output.stdout.len() > 10 * 1024 * 1024 {
                return Err(());
            }
            Ok(output.stdout)
        })
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;
        tokio::fs::write(&path, bytes).await.map_err(|_| ())?;
        // Restrict the temp before the backend streams it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| ())?;
        }
        return Ok(WaylandClipboard::File {
            path,
            mime: mime.to_owned(),
        });
    }
    let offered = offered.iter().map(String::as_str).collect::<Vec<_>>();
    let html = if offered.contains(&"text/html") {
        Some(
            crate::clipboard::read_text_mime("text/html")
                .await
                .ok_or(())?,
        )
    } else {
        None
    };
    let uri = if offered.contains(&"text/uri-list") {
        Some(
            crate::clipboard::read_text_mime("text/uri-list")
                .await
                .ok_or(())?,
        )
    } else {
        None
    };
    if let Some(uri) = &uri {
        if let Some(path) = uri.lines().map(str::trim).find_map(|line| {
            Url::parse(line)
                .ok()
                .and_then(|url| url.to_file_path().ok())
        }) {
            let metadata = fs::metadata(&path).map_err(|_| ())?;
            if metadata.is_file() && metadata.len() <= 10 * 1024 * 1024 {
                let name = path.file_name().and_then(|name| name.to_str()).ok_or(())?;
                let extension = Path::new(name)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or("octet-stream");
                let mime = match extension.to_ascii_lowercase().as_str() {
                    "png" => "image/png",
                    "jpg" | "jpeg" => "image/jpeg",
                    "gif" => "image/gif",
                    "webp" => "image/webp",
                    "pdf" => "application/pdf",
                    _ => "application/octet-stream",
                };
                let copy = runtime_directory().map_err(|_| ())?.join(format!(
                    "handover-clipboard-{}-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_err(|_| ())?
                        .as_nanos(),
                    name
                ));
                // Private runtime temp, created exclusively as 0600:
                // clipboard-selected files must never pass through
                // the shared temp directory.
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    let mut staged = std::fs::OpenOptions::new();
                    staged.write(true).create_new(true).mode(0o600);
                    let mut staged = staged.open(&copy).map_err(|_| ())?;
                    let mut source = std::fs::File::open(&path).map_err(|_| ())?;
                    std::io::copy(&mut source, &mut staged).map_err(|_| ())?;
                }
                return Ok(WaylandClipboard::File {
                    path: copy,
                    mime: mime.to_owned(),
                });
            }
        }
    }
    let plain_mime = offered
        .iter()
        .copied()
        .find(|mime| *mime == "text/plain;charset=utf-8")
        .or_else(|| offered.iter().copied().find(|mime| *mime == "text/plain"));
    let text = if let Some(mime) = plain_mime {
        crate::clipboard::read_text_mime(mime).await.ok_or(())?
    } else if let Some(html) = &html {
        strip_html_text(html)
    } else if let Some(uri) = &uri {
        uri.clone()
    } else {
        return Err(());
    };
    if text.len() + html.as_ref().map_or(0, String::len) + uri.as_ref().map_or(0, String::len)
        > 60 * 1024
    {
        return Err(());
    }
    Ok(WaylandClipboard::Text { text, html, uri })
}

pub(crate) fn strip_html_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    for character in html.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(character),
            _ => {}
        }
    }
    text
}
