//! One receive owner with bounded reads and publication-before-ACK for pushes.
use super::*;
use handover_core::messaging::Conversation;
use tokio::sync::{mpsc, oneshot};

pub enum LiveCommand {
    Conversations,
    History {
        conversation: Box<Conversation>,
        cursor: Option<Zeroizing<String>>,
        limit: u32,
        fetch_id: Option<u64>,
    },
}
pub enum LiveEvent {
    Online,
    Conversations(Vec<Conversation>),
    ConversationUpdates(Vec<Conversation>),
    History {
        conversation: String,
        fetch_id: Option<u64>,
        page: crate::history::HistoryPage,
    },
    Messages(Vec<handover_core::messaging::Message>),
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
        let mut known = Vec::new();
        let mut pending = Some(LiveCommand::Conversations);
        let mut online = false;
        loop {
            if let Some(command) = pending.take() {
                let (conversations, history, fetch_id) = match &command {
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
                let read = tokio::time::timeout(
                    Duration::from_secs(if conversations { 120 } else { 30 }),
                    self.read_on_stream(
                        &short,
                        endpoint,
                        conversations,
                        history,
                        &correlation,
                        &mut reply_rx,
                    ),
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
                match result {
                    ReadResult::Conversations(records) => {
                        merge(&mut known, &records)?;
                        publish(&events, LiveEvent::Conversations(records)).await?;
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
                    ReadResult::Startup => return Err(ProbeError::UnexpectedResponse),
                }
                *correlation.lock().map_err(|_| ProbeError::NativeError)? =
                    (Zeroizing::new(String::new()), 0);
            } else {
                tokio::select! {
                    result = &mut receive => { result?; return Err(ProbeError::ReceiveFailed); }
                    _ = &mut expiry => return Err(ProbeError::SessionExpired),
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
            crate::updates::Update::Conversations(records) => {
                merge(known, &records)?;
                publish(events, LiveEvent::ConversationUpdates(records)).await?;
            }
            crate::updates::Update::Messages(records) => {
                publish(events, LiveEvent::Messages(records)).await?
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
