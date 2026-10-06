//! One receive owner with bounded reads and publication-before-ACK for pushes.
use super::*;
use handover_core::messaging::Conversation;
use tokio::sync::{mpsc, oneshot};

pub enum LiveCommand {
    SendingCapability,
    SendText {
        request_id: String,
        conversation: String,
        text: Zeroizing<String>,
        accepted: oneshot::Receiver<()>,
    },
    SendMedia {
        request_id: String,
        conversation: String,
        path: Zeroizing<String>,
        caption: Option<Zeroizing<String>>,
        accepted: oneshot::Receiver<()>,
    },
    Conversations,
    History {
        conversation: Box<Conversation>,
        cursor: Option<Zeroizing<String>>,
        limit: u32,
        fetch_id: Option<u64>,
    },
}
pub enum LiveEvent {
    SendResult {
        request_id: String,
        conversation: String,
        message: Option<String>,
        status: &'static str,
    },
    Online,
    Conversations(Vec<Conversation>),
    ConversationUpdates(Vec<Conversation>),
    History {
        conversation: String,
        fetch_id: Option<u64>,
        page: crate::history::HistoryPage,
    },
    Messages {
        records: Vec<handover_core::messaging::Message>,
        statuses: Vec<(handover_core::messaging::MessageId, &'static str)>,
        correlations: Vec<(String, handover_core::messaging::MessageId)>,
    },
}
/// The consumer confirms publication before a processed push is acknowledged.
pub struct LiveOutput {
    pub event: LiveEvent,
    pub accepted: oneshot::Sender<()>,
}
impl fmt::Debug for LiveOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LiveOutput { redacted }")
    }
}
async fn publish(events: &mpsc::Sender<LiveOutput>, event: LiveEvent) -> Result<(), ProbeError> {
    let (accepted, processed) = oneshot::channel();
    events
        .try_send(LiveOutput { event, accepted })
        .map_err(|_| ProbeError::NativeError)?;
    tokio::time::timeout(Duration::from_secs(30), processed)
        .await
        .map_err(|_| ProbeError::Timeout)?
        .map_err(|_| ProbeError::NativeError)
}
fn media_failure(error: crate::media::MediaError) -> &'static str {
    use crate::media::MediaError;
    match error {
        MediaError::InvalidFile | MediaError::Empty => "failed:rejected",
        MediaError::TooLarge => "failed:too_large",
        MediaError::Authentication
        | MediaError::Randomness
        | MediaError::EncryptionKeyUnavailable => "failed:encryption",
        _ => "failed:transport",
    }
}
fn merge(known: &mut Vec<Conversation>, records: &[Conversation]) -> Result<(), ProbeError> {
    for record in records {
        if let Some(previous) = known.iter_mut().find(|previous| previous.id == record.id) {
            *previous = record.clone();
        } else {
            if known.len() >= 10_000 {
                return Err(ProbeError::SessionProtocol(SessionError::PaginationLimit));
            }
            known.push(record.clone());
        }
    }
    let mut size = 0usize;
    for record in known {
        size += serde_json::to_vec(record)
            .map_err(|_| ProbeError::NativeError)?
            .len();
        if size > 16 * 1024 * 1024 {
            return Err(ProbeError::SessionProtocol(SessionError::PaginationLimit));
        }
    }
    Ok(())
}
impl RecoveredSession {
    async fn sending_capability(
        &self,
        http: &reqwest::Client,
        endpoint: &str,
        correlation: &std::sync::Mutex<(Zeroizing<String>, i32)>,
        replies: &mut mpsc::Receiver<crate::receive::session_reply::SessionReply>,
    ) -> Result<bool, ProbeError> {
        let id = Uuid::new_v4().to_string();
        *correlation.lock().map_err(|_| ProbeError::NativeError)? =
            (Zeroizing::new(id.clone()), 31);
        let request = self.build_request(&id, 31, &[], 2, Some(86_400_000_000))?;
        self.post_request(http, endpoint, &request).await?;
        let reply = replies.recv().await.ok_or(ProbeError::ReceiveFailed)?;
        let plain = self
            .pairing
            .pairing
            .decrypt_payload(&reply.ciphertext)
            .map_err(|_| ProbeError::SessionProtocol(SessionError::UpdateAuthentication))?;
        let allowed = crate::send::capability(plain.as_bytes())?;
        self.acknowledge(http, endpoint, reply.message_id).await?;
        Ok(allowed)
    }
    async fn send_on_stream(
        &self,
        http: &reqwest::Client,
        endpoint: &str,
        payload: &[u8],
        correlation: &std::sync::Mutex<(Zeroizing<String>, i32)>,
        replies: &mut mpsc::Receiver<crate::receive::session_reply::SessionReply>,
    ) -> Result<(crate::send::Reply, Zeroizing<String>), ProbeError> {
        let id = Uuid::new_v4().to_string();
        *correlation.lock().map_err(|_| ProbeError::NativeError)? = (Zeroizing::new(id.clone()), 3);
        let request = self.build_request(&id, 3, payload, 2, Some(60_000_000))?;
        self.post_request(http, endpoint, &request).await?;
        let reply = replies.recv().await.ok_or(ProbeError::ReceiveFailed)?;
        let plain = self
            .pairing
            .pairing
            .decrypt_payload(&reply.ciphertext)
            .map_err(|_| ProbeError::SessionProtocol(SessionError::UpdateAuthentication))?;
        Ok((crate::send::reply(plain.as_bytes())?, reply.message_id))
    }
    pub async fn run_live(
        &self,
        commands: mpsc::Receiver<LiveCommand>,
        events: mpsc::Sender<LiveOutput>,
    ) -> Result<(), ProbeError> {
        self.run_live_at(
            &self.proof.endpoint,
            crate::client(true)?,
            crate::streaming_client()?,
            commands,
            events,
            None,
        )
        .await
    }
    pub(crate) async fn run_live_at(
        &self,
        endpoint: &str,
        short: reqwest::Client,
        stream: reqwest::Client,
        mut commands: mpsc::Receiver<LiveCommand>,
        events: mpsc::Sender<LiveOutput>,
        renewal_after: Option<Duration>,
    ) -> Result<(), ProbeError> {
        let sources = Zeroizing::new(
            crate::query_response(
                &short,
                &format!("{endpoint}{}", crate::SIGN_IN_PATH),
                self.proof.validate_pairing()?,
                &self.pairing.registration.lookup_request(),
                false,
            )
            .await?,
        );
        let sources = crate::sources::RegisteredSources::from_lookup_response(&sources)?;
        if !sources.contains_registration(&self.pairing.registration) {
            return Err(ProbeError::RegistrationAccountMismatch);
        }
        if !sources.contains_paired_phone(self.pairing.pairing.peer()) {
            return Err(ProbeError::NoEligiblePhone);
        }
        let request = self
            .pairing
            .registration
            .prepare_receive()
            .map_err(|_| ProbeError::SessionExpired)?;
        let activation = self.prepare_activation()?;
        let correlation = std::sync::Mutex::new((Zeroizing::new(String::new()), 0));
        let session_id = self.session_id.to_string();
        let (reply_tx, mut reply_rx) = mpsc::channel(1);
        let (push_tx, mut push_rx) = mpsc::channel(4);
        let (ready_tx, ready_rx) = oneshot::channel();
        let receive_url = format!("{endpoint}{}", crate::RECEIVE_MESSAGES_PATH);
        let receive = crate::receive_stream_when_ready(
            &stream,
            &receive_url,
            self.proof.validate_messaging()?,
            &request,
            |event| {
                if let crate::receive::ReceiveEvent::Record(record) = event {
                    let pending = correlation
                        .lock()
                        .map_err(|_| crate::receive::ReceiveError::TooLarge)?;
                    if let Some(reply) =
                        record.session_reply(&pending.0, pending.1, self.pairing.pairing.peer())?
                    {
                        reply_tx
                            .try_send(reply)
                            .map_err(|_| crate::receive::ReceiveError::TooLarge)?;
                    } else if let Some(push) =
                        record.session_reply(&session_id, 16, self.pairing.pairing.peer())?
                    {
                        push_tx
                            .try_send(push)
                            .map_err(|_| crate::receive::ReceiveError::TooLarge)?;
                    }
                }
                Ok(())
            },
            || {
                let _ = ready_tx.send(());
            },
        );
        tokio::pin!(receive);
        let startup = async {
            ready_rx.await.map_err(|_| ProbeError::ReceiveFailed)?;
            self.post_request(&short, endpoint, &activation).await
        };
        tokio::select! {
            result = &mut receive => { result?; return Err(ProbeError::ReceiveFailed); }
            result = tokio::time::timeout(Duration::from_secs(30), startup) => { result.map_err(|_| ProbeError::Timeout)??; }
        }
        let lifetime = self
            .pairing
            .registration
            .remaining_lifetime()
            .map_err(|_| ProbeError::SessionExpired)?;
        let expiry = tokio::time::sleep(lifetime);
        tokio::pin!(expiry);
        let renewal = tokio::time::sleep(renewal_after.unwrap_or_else(|| renewal_delay(lifetime)));
        tokio::pin!(renewal);
        let mut known = Vec::<Conversation>::new();
        let mut pending = Some(LiveCommand::Conversations);
        let mut online = false;
        let mut can_send = false;
        loop {
            // Let the current request and its publication finish before rotating
            // receive ownership. Never cancel or replay an active send to renew.
            if pending.is_none() && renewal.is_elapsed() {
                return Err(ProbeError::RegistrationRenewalDue);
            }
            if let Some(command) = pending.take() {
                if matches!(
                    &command,
                    LiveCommand::SendText { .. } | LiveCommand::SendMedia { .. }
                ) {
                    let (request_id, conversation, text, media, accepted) = match command {
                        LiveCommand::SendText {
                            request_id,
                            conversation,
                            text,
                            accepted,
                        } => (request_id, conversation, Some(text), None, accepted),
                        LiveCommand::SendMedia {
                            request_id,
                            conversation,
                            path,
                            caption,
                            accepted,
                        } => (
                            request_id,
                            conversation,
                            None,
                            Some((path, caption)),
                            accepted,
                        ),
                        _ => unreachable!(),
                    };
                    accepted.await.map_err(|_| ProbeError::NativeError)?;
                    if !online
                        || !can_send
                        || !known.iter().any(|model| model.id.local_id == conversation)
                    {
                        publish(
                            &events,
                            LiveEvent::SendResult {
                                request_id,
                                conversation,
                                message: None,
                                status: "failed:rejected",
                            },
                        )
                        .await?;
                        continue;
                    }
                    let temporary = crate::send::temporary(&request_id).to_string();
                    let payload = if let Some(text) = text {
                        crate::send::request(&conversation, &text, &temporary)?
                    } else {
                        let (path, caption) = media.ok_or(ProbeError::NativeError)?;
                        let caption = caption.as_deref().map(String::as_str);
                        if crate::send::validate_media(&conversation, caption).is_err() {
                            publish(
                                &events,
                                LiveEvent::SendResult {
                                    request_id,
                                    conversation,
                                    message: None,
                                    status: "failed:rejected",
                                },
                            )
                            .await?;
                            continue;
                        }
                        // Uploading alone cannot send a phone message. Failures
                        // here have a known outcome and are never retried.
                        let upload = tokio::time::timeout(
                            Duration::from_secs(60),
                            crate::media::upload_staged(&self.pairing.registration, path),
                        );
                        tokio::pin!(upload);
                        let uploaded = loop {
                            tokio::select! {
                                result = &mut receive => { result?; return Err(ProbeError::ReceiveFailed); }
                                _ = &mut expiry => return Err(ProbeError::SessionExpired),
                                result = &mut upload => break result.unwrap_or(Err(crate::media::MediaError::Network)),
                                push = push_rx.recv() => self.apply_live_push(&short, endpoint, &events, &mut known, &mut online, push.ok_or(ProbeError::ReceiveFailed)?).await?,
                            }
                        };
                        let uploaded = match uploaded {
                            Ok(uploaded) => uploaded,
                            Err(error) => {
                                publish(
                                    &events,
                                    LiveEvent::SendResult {
                                        request_id,
                                        conversation,
                                        message: None,
                                        status: media_failure(error),
                                    },
                                )
                                .await?;
                                continue;
                            }
                        };
                        crate::send::media_request(&conversation, &uploaded, caption, &temporary)?
                    };
                    let send = tokio::time::timeout(
                        Duration::from_secs(60),
                        self.send_on_stream(
                            &short,
                            endpoint,
                            &payload,
                            &correlation,
                            &mut reply_rx,
                        ),
                    );
                    tokio::pin!(send);
                    let (reply, inbox_id) = loop {
                        tokio::select! {
                            result = &mut receive => { result?; return Err(ProbeError::ReceiveFailed); }
                            _ = &mut expiry => return Err(ProbeError::SessionExpired),
                            result = &mut send => break result.map_err(|_| ProbeError::Timeout)??,
                            push = push_rx.recv() => self.apply_live_push(&short, endpoint, &events, &mut known, &mut online, push.ok_or(ProbeError::ReceiveFailed)?).await?,
                        }
                    };
                    let (message, status) = match reply {
                        crate::send::Reply::Accepted(id) => (id, "accepted"),
                        crate::send::Reply::Rejected => (None, "failed:rejected"),
                    };
                    publish(
                        &events,
                        LiveEvent::SendResult {
                            request_id,
                            conversation,
                            message,
                            status,
                        },
                    )
                    .await?;
                    self.acknowledge(&short, endpoint, inbox_id).await?;
                    *correlation.lock().map_err(|_| ProbeError::NativeError)? =
                        (Zeroizing::new(String::new()), 0);
                    continue;
                }
                let (conversations, history, fetch_id) = match &command {
                    LiveCommand::SendText { .. } => unreachable!(),
                    LiveCommand::SendMedia { .. } => unreachable!(),
                    LiveCommand::SendingCapability => (false, None, None),
                    LiveCommand::Conversations => (true, None, None),
                    LiveCommand::History {
                        conversation,
                        cursor,
                        limit,
                        fetch_id,
                    } => {
                        if conversation.id.account_id.as_str() != self.account_id() {
                            return Err(ProbeError::RegistrationAccountMismatch);
                        }
                        (
                            false,
                            Some((
                                conversation.as_ref(),
                                crate::history::request(
                                    conversation,
                                    cursor.as_deref().map(String::as_str),
                                    *limit,
                                )?,
                            )),
                            *fetch_id,
                        )
                    }
                };
                let read_started = tokio::time::Instant::now();
                let read = tokio::time::timeout(
                    Duration::from_secs(if conversations { 120 } else { 30 }),
                    async {
                        let mut result = self
                            .read_on_stream(
                                &short,
                                endpoint,
                                conversations,
                                history.map(|(conversation, payload)| {
                                    (
                                        conversation,
                                        payload,
                                        read_started + Duration::from_secs(30),
                                    )
                                }),
                                &correlation,
                                &mut reply_rx,
                            )
                            .await?;
                        if let ReadResult::History(page) = &mut result {
                            // Optional file enrichment must not turn a valid
                            // slow history reply into a timed-out request.
                            let budget = Duration::from_secs(30)
                                .saturating_sub(read_started.elapsed())
                                .saturating_sub(Duration::from_millis(500));
                            crate::media::hydrate_page(&self.pairing.registration, page, budget)
                                .await;
                        }
                        let allowed = if matches!(command, LiveCommand::SendingCapability) {
                            Some(
                                self.sending_capability(
                                    &short,
                                    endpoint,
                                    &correlation,
                                    &mut reply_rx,
                                )
                                .await?,
                            )
                        } else {
                            None
                        };
                        Ok::<_, ProbeError>((result, allowed))
                    },
                );
                tokio::pin!(read);
                let result = loop {
                    tokio::select! {
                        result = &mut receive => { result?; return Err(ProbeError::ReceiveFailed); }
                        _ = &mut expiry => return Err(ProbeError::SessionExpired),
                        result = &mut read => break result.map_err(|_| ProbeError::Timeout)??,
                        // The initial inventory supplies participant identities.
                        // Its bounded startup pushes remain queued until then.
                        push = push_rx.recv(), if !known.is_empty() => {
                            self.apply_live_push(&short, endpoint, &events, &mut known, &mut online, push.ok_or(ProbeError::ReceiveFailed)?).await?;
                        }
                    }
                };
                if let Some(allowed) = result.1 {
                    can_send = allowed;
                }
                match result.0 {
                    ReadResult::Conversations(mut records) => {
                        for record in &mut records {
                            if can_send {
                                record.capabilities.extend([
                                    handover_core::messaging::MessagingCapability::Text,
                                    handover_core::messaging::MessagingCapability::Media,
                                ]);
                            }
                        }
                        merge(&mut known, &records)?;
                        publish(&events, LiveEvent::Conversations(records)).await?;
                        pending = Some(LiveCommand::SendingCapability);
                    }
                    ReadResult::History(page) => {
                        let LiveCommand::History { conversation, .. } = &command else {
                            return Err(ProbeError::UnexpectedResponse);
                        };
                        publish(
                            &events,
                            LiveEvent::History {
                                conversation: conversation.id.local_id.clone(),
                                fetch_id,
                                page,
                            },
                        )
                        .await?;
                    }
                    ReadResult::Startup if matches!(command, LiveCommand::SendingCapability) => {
                        for model in &mut known {
                            if can_send {
                                model.capabilities.extend([
                                    handover_core::messaging::MessagingCapability::Text,
                                    handover_core::messaging::MessagingCapability::Media,
                                ]);
                            } else {
                                model
                                    .capabilities
                                    .remove(&handover_core::messaging::MessagingCapability::Text);
                                model
                                    .capabilities
                                    .remove(&handover_core::messaging::MessagingCapability::Media);
                            }
                        }
                        publish(&events, LiveEvent::ConversationUpdates(known.clone())).await?;
                    }
                    ReadResult::Startup => return Err(ProbeError::UnexpectedResponse),
                }
                *correlation.lock().map_err(|_| ProbeError::NativeError)? =
                    (Zeroizing::new(String::new()), 0);
            } else {
                tokio::select! {
                    result = &mut receive => { result?; return Err(ProbeError::ReceiveFailed); }
                    _ = &mut expiry => return Err(ProbeError::SessionExpired),
                    _ = &mut renewal => return Err(ProbeError::RegistrationRenewalDue),
                    command = commands.recv() => {
                        let Some(command) = command else { return Ok(()); };
                        pending = Some(command);
                    }
                    push = push_rx.recv() => {
                        self.apply_live_push(&short, endpoint, &events, &mut known, &mut online, push.ok_or(ProbeError::ReceiveFailed)?).await?;
                    }
                }
            }
        }
    }
    async fn apply_live_push(
        &self,
        http: &reqwest::Client,
        endpoint: &str,
        events: &mpsc::Sender<LiveOutput>,
        known: &mut Vec<Conversation>,
        online: &mut bool,
        push: crate::receive::session_reply::SessionReply,
    ) -> Result<(), ProbeError> {
        let can_send = known.iter().any(|model| {
            model
                .capabilities
                .contains(&handover_core::messaging::MessagingCapability::Text)
        });
        let plaintext = self
            .pairing
            .pairing
            .decrypt_payload(&push.ciphertext)
            .map_err(|_| ProbeError::SessionProtocol(SessionError::UpdateAuthentication))?;
        match crate::updates::decode(self.account_id(), known, plaintext.as_bytes(), true)? {
            crate::updates::Update::Active => {
                if !*online {
                    publish(events, LiveEvent::Online).await?;
                    *online = true;
                }
            }
            crate::updates::Update::Inactive => {
                return Err(ProbeError::ReceiveProtocol(
                    crate::receive::ReceiveError::SessionPreempted,
                ));
            }
            crate::updates::Update::Conversations(mut records) => {
                for record in &mut records {
                    if can_send {
                        record.capabilities.extend([
                            handover_core::messaging::MessagingCapability::Text,
                            handover_core::messaging::MessagingCapability::Media,
                        ]);
                    }
                }
                merge(known, &records)?;
                publish(events, LiveEvent::ConversationUpdates(records)).await?;
            }
            crate::updates::Update::Messages {
                records,
                statuses,
                correlations,
            } => {
                publish(
                    events,
                    LiveEvent::Messages {
                        records,
                        statuses,
                        correlations,
                    },
                )
                .await?
            }
            crate::updates::Update::PresenceCheck => {
                let request = self.build_request(
                    &Uuid::new_v4().to_string(),
                    17,
                    &[],
                    2,
                    Some(86_400_000_000),
                )?;
                self.post_request(http, endpoint, &request).await?;
            }
            // Unsupported families are left in the inbox, without pretending
            // their contents were processed or erasing them with an ACK.
            crate::updates::Update::Unsupported(_) => return Ok(()),
        }
        self.acknowledge(http, endpoint, push.message_id).await
    }
}
