//! Restored credentials and encrypted session activation, without online claims.
use crate::{
    BrowserProof, ProbeError, credential_store::DesktopCredentialStore, login::ConfirmedPairing,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use prost::Message;
use serde_json::{Value, json};
use std::{
    fmt,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
mod live;
pub use live::{LiveCommand, LiveEvent, LiveOutput};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    SendModel,
    ActivationTransport,
    UpdateModel,
    UpdateAuthentication,
    UpdateStreamEnded,
    UnknownUpdateConversation,
    ConversationTransport,
    ConversationEncoding,
    ConversationCount,
    EmptyConversation,
    OversizedConversation,
    ConversationCursor,
    PaginationLimit,
    HistoryModel,
    UnsupportedHistoryContent,
    ConversationModel,
    ParticipantIdentity,
    ParticipantAddressConflict,
    InvalidUnreadCount,
    ConversationText,
    ConversationIdentifier,
    TooManyParticipants,
    MissingParticipants,
    DuplicateParticipants,
    DuplicateConversations,
    ParticipantRoleConflict,
    ParticipantNameConflict,
    DirectParticipants,
}
fn stage(error: ProbeError, category: SessionError) -> ProbeError {
    if matches!(error, ProbeError::UnexpectedResponse) {
        ProbeError::SessionProtocol(category)
    } else {
        error
    }
}

pub(crate) enum ReadResult {
    Startup,
    Conversations(Vec<handover_core::messaging::Conversation>),
    History(crate::history::HistoryPage),
}

fn unique_conversations(
    records: Vec<handover_core::messaging::Conversation>,
) -> Result<Vec<handover_core::messaging::Conversation>, ProbeError> {
    let mut unique = Vec::<handover_core::messaging::Conversation>::new();
    for record in records {
        if let Some(previous) = unique.iter().find(|previous| previous.id == record.id) {
            if previous != &record {
                return Err(ProbeError::SessionProtocol(
                    SessionError::DuplicateConversations,
                ));
            }
        } else {
            unique.push(record);
        }
    }
    Ok(unique)
}

const MAX_PAGE_RECORDS: usize = 1024;

pub struct RecoveredSession {
    pairing: ConfirmedPairing,
    proof: BrowserProof,
    session_id: Uuid,
}
impl fmt::Debug for RecoveredSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecoveredSession { redacted }")
    }
}
impl RecoveredSession {
    /// Loading secrets does not contact Google or establish an online account.
    pub async fn restore(pairing: ConfirmedPairing) -> Result<Self, ProbeError> {
        let proof = DesktopCredentialStore::load(pairing.account_id())
            .await
            .map_err(ProbeError::CredentialStore)?
            .ok_or(ProbeError::InvalidCredentials)?;
        Self::from_credentials(pairing, proof)
    }
    pub(crate) fn from_credentials(
        pairing: ConfirmedPairing,
        proof: BrowserProof,
    ) -> Result<Self, ProbeError> {
        proof.validate_pairing()?;
        pairing
            .registration
            .remaining_lifetime()
            .map_err(|_| ProbeError::SessionExpired)?;
        Ok(Self {
            pairing,
            proof,
            session_id: Uuid::new_v4(),
        })
    }
    pub fn account_id(&self) -> &str {
        self.pairing.account_id()
    }

    /// One bounded startup attempt. Opening receive and HTTP acceptance of the
    /// activation request are transport observations, not an online account.
    pub async fn probe_startup(&self) -> Result<(), ProbeError> {
        tokio::time::timeout(
            Duration::from_secs(30),
            self.probe_startup_at(
                &self.proof.endpoint,
                crate::client(true)?,
                crate::streaming_client()?,
                false,
            ),
        )
        .await
        .map_err(|_| ProbeError::Timeout)?
        .map(|_| ())
    }

    pub(crate) async fn probe_startup_at(
        &self,
        endpoint: &str,
        short: reqwest::Client,
        stream: reqwest::Client,
        conversations: bool,
    ) -> Result<Option<Vec<handover_core::messaging::Conversation>>, ProbeError> {
        match self
            .run_at(endpoint, short, stream, conversations, None)
            .await?
        {
            ReadResult::Startup => Ok(None),
            ReadResult::Conversations(records) => Ok(Some(records)),
            ReadResult::History(_) => Err(ProbeError::UnexpectedResponse),
        }
    }

    pub async fn read_history(
        &self,
        conversation: &handover_core::messaging::Conversation,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<crate::history::HistoryPage, ProbeError> {
        if conversation.id.account_id.as_str() != self.account_id() {
            return Err(ProbeError::RegistrationAccountMismatch);
        }
        let payload = crate::history::request(conversation, cursor, limit)?;
        let result = self
            .run_at(
                &self.proof.endpoint,
                crate::client(true)?,
                crate::streaming_client()?,
                false,
                Some((conversation, payload)),
            )
            .await?;
        match result {
            ReadResult::History(page) => Ok(page),
            _ => Err(ProbeError::UnexpectedResponse),
        }
    }

    #[cfg(test)]
    pub(crate) async fn history_at(
        &self,
        endpoint: &str,
        http: reqwest::Client,
        conversation: &handover_core::messaging::Conversation,
    ) -> Result<crate::history::HistoryPage, ProbeError> {
        let payload = crate::history::request(conversation, None, 20)?;
        match self
            .run_at(
                endpoint,
                http.clone(),
                http,
                false,
                Some((conversation, payload)),
            )
            .await?
        {
            ReadResult::History(page) => Ok(page),
            _ => Err(ProbeError::UnexpectedResponse),
        }
    }

    async fn run_at(
        &self,
        endpoint: &str,
        short: reqwest::Client,
        stream: reqwest::Client,
        conversations: bool,
        history: Option<(&handover_core::messaging::Conversation, Zeroizing<Vec<u8>>)>,
    ) -> Result<ReadResult, ProbeError> {
        let action = if history.is_some() {
            2
        } else if conversations {
            1
        } else {
            0
        };
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
        let list_id = std::sync::Mutex::new((Zeroizing::new(String::new()), action));
        let (reply_tx, mut reply_rx) = tokio::sync::mpsc::channel(1);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let receive_url = format!("{endpoint}{}", crate::RECEIVE_MESSAGES_PATH);
        let receive = crate::receive_stream_when_ready(
            &stream,
            &receive_url,
            self.proof.validate_messaging()?,
            &request,
            |event| {
                if action != 0 {
                    if let crate::receive::ReceiveEvent::Record(record) = event {
                        if let Some(reply) = record.session_reply(
                            &list_id
                                .lock()
                                .map_err(|_| crate::receive::ReceiveError::TooLarge)?
                                .0,
                            action,
                            self.pairing.pairing.peer(),
                        )? {
                            reply_tx
                                .try_send(reply)
                                .map_err(|_| crate::receive::ReceiveError::TooLarge)?;
                        }
                    }
                }
                Ok(())
            },
            || {
                let _ = tx.send(());
            },
        );
        tokio::pin!(receive);
        let attempt = async {
            tokio::select! {
                result = &mut receive => { result?; return Err(ProbeError::ReceiveFailed); }
                ready = rx => { ready.map_err(|_| ProbeError::ReceiveFailed)?; }
            }
            let send = async {
                self.post_request(&short, endpoint, &activation)
                    .await
                    .map_err(|error| stage(error, SessionError::ActivationTransport))?;
                self.read_on_stream(
                    &short,
                    endpoint,
                    conversations,
                    history,
                    &list_id,
                    &mut reply_rx,
                )
                .await
            };
            tokio::select! {
                result = &mut receive => { result?; Err(ProbeError::ReceiveFailed) }
                result = send => result,
            }
        };
        tokio::time::timeout(
            Duration::from_secs(if conversations { 120 } else { 30 }),
            attempt,
        )
        .await
        .map_err(|_| ProbeError::Timeout)?
    }

    async fn read_on_stream(
        &self,
        short: &reqwest::Client,
        endpoint: &str,
        conversations: bool,
        history: Option<(&handover_core::messaging::Conversation, Zeroizing<Vec<u8>>)>,
        list_id: &std::sync::Mutex<(Zeroizing<String>, i32)>,
        reply_rx: &mut tokio::sync::mpsc::Receiver<crate::receive::session_reply::SessionReply>,
    ) -> Result<ReadResult, ProbeError> {
        if let Some((conversation, payload)) = history {
            let request_id = Uuid::new_v4().to_string();
            *list_id.lock().map_err(|_| ProbeError::ReceiveFailed)? =
                (Zeroizing::new(request_id.clone()), 2);
            let request = self.build_request(&request_id, 2, &payload, 2, Some(86_400_000_000))?;
            self.post_request(short, endpoint, &request).await?;
            let reply = reply_rx.recv().await.ok_or(ProbeError::ReceiveFailed)?;
            let plaintext = self
                .pairing
                .pairing
                .decrypt_payload(&reply.ciphertext)
                .map_err(|_| ProbeError::ReceiveFailed)?;
            let page = crate::history::decode(conversation, plaintext.as_bytes())?;
            self.acknowledge(short, endpoint, reply.message_id).await?;
            return Ok(ReadResult::History(page));
        }
        if !conversations {
            return Ok(ReadResult::Startup);
        }
        let mut next_cursor = None;
        let mut cursors = Vec::new();
        let mut all = Vec::new();
        let mut total_records = 0;
        let mut total_bytes = 0;
        for page_index in 0..100 {
            let payload = Zeroizing::new(
                ConversationRequest {
                    limit: 25,
                    status: 1,
                    cursor: next_cursor.take(),
                }
                .encode_to_vec(),
            );
            let request_id = Uuid::new_v4().to_string();
            *list_id.lock().map_err(|_| ProbeError::ReceiveFailed)? =
                (Zeroizing::new(request_id.clone()), 1);
            let list = self.build_request(
                &request_id,
                1,
                &payload,
                if page_index == 0 { 16 } else { 2 },
                Some(86_400_000_000),
            )?;
            self.post_request(short, endpoint, &list)
                .await
                .map_err(|error| stage(error, SessionError::ConversationTransport))?;
            let reply = reply_rx.recv().await.ok_or(ProbeError::ReceiveFailed)?;
            let plaintext = self
                .pairing
                .pairing
                .decrypt_payload(&reply.ciphertext)
                .map_err(|_| ProbeError::ReceiveFailed)?;
            let mut page = ConversationPage::decode(plaintext.as_bytes())
                .map_err(|_| ProbeError::SessionProtocol(SessionError::ConversationEncoding))?;
            if page.conversations.len() > MAX_PAGE_RECORDS {
                return Err(ProbeError::SessionProtocol(SessionError::ConversationCount));
            }
            if page.conversations.iter().any(Vec::is_empty) {
                return Err(ProbeError::SessionProtocol(SessionError::EmptyConversation));
            }
            if page
                .conversations
                .iter()
                .any(|record| record.len() > 64 * 1024)
            {
                return Err(ProbeError::SessionProtocol(
                    SessionError::OversizedConversation,
                ));
            }
            if let Some(cursor) = &page.cursor {
                if cursor.id.is_empty()
                    || cursor.id.len() > 1024
                    || cursor.id.chars().any(char::is_control)
                    || cursor.timestamp < 0
                {
                    return Err(ProbeError::SessionProtocol(
                        SessionError::ConversationCursor,
                    ));
                }
            }
            let decoded = page
                .conversations
                .iter()
                .map(|bytes| crate::conversation::decode(self.account_id(), bytes))
                .collect::<Result<Vec<_>, _>>()?;
            total_records += decoded.len();
            total_bytes += plaintext.as_bytes().len();
            if total_records > 10_000 || total_bytes > 16 * 1024 * 1024 {
                return Err(ProbeError::SessionProtocol(SessionError::PaginationLimit));
            }
            all.extend(decoded);
            all = unique_conversations(all)?;
            next_cursor = page.cursor.take();
            if let Some(cursor) = &next_cursor {
                if cursors.iter().any(|seen: &ConversationCursor| {
                    seen.id == cursor.id && seen.timestamp == cursor.timestamp
                }) {
                    return Err(ProbeError::SessionProtocol(
                        SessionError::ConversationCursor,
                    ));
                }
                cursors.push(ConversationCursor {
                    id: cursor.id.clone(),
                    timestamp: cursor.timestamp,
                });
            }

            self.acknowledge(short, endpoint, reply.message_id).await?;
            if next_cursor.is_none() {
                return Ok(ReadResult::Conversations(all));
            }
        }
        Err(ProbeError::SessionProtocol(SessionError::PaginationLimit))
    }

    /// Keep one receive stream open for a bounded, read-only diagnostic.
    /// Pushes are authenticated and projected but never acknowledged here.
    /// This does not establish an online account or automatic reconnect policy.
    pub async fn observe_updates(
        &self,
        known: Vec<handover_core::messaging::Conversation>,
        duration: Duration,
        emit: impl FnMut(crate::updates::Update) -> Result<(), ProbeError>,
    ) -> Result<(), ProbeError> {
        if duration.is_zero() || duration > Duration::from_secs(120) {
            return Err(ProbeError::InvalidBootstrap);
        }
        self.observe_updates_at(
            &self.proof.endpoint,
            crate::client(true)?,
            crate::streaming_client()?,
            known,
            duration,
            emit,
        )
        .await
    }

    pub(crate) async fn observe_updates_at(
        &self,
        endpoint: &str,
        short: reqwest::Client,
        stream: reqwest::Client,
        mut known: Vec<handover_core::messaging::Conversation>,
        duration: Duration,
        mut emit: impl FnMut(crate::updates::Update) -> Result<(), ProbeError>,
    ) -> Result<(), ProbeError> {
        if known.len() > 10_000
            || known.iter().any(|record| {
                record.id.account_id.as_str() != self.account_id()
                    || handover_core::messaging::validate_conversation(record).is_err()
            })
        {
            return Err(ProbeError::SessionProtocol(SessionError::UpdateModel));
        }
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
        let session_id = self.session_id.to_string();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (push_tx, mut push_rx) = tokio::sync::mpsc::channel(4);
        let receive_url = format!("{endpoint}{}", crate::RECEIVE_MESSAGES_PATH);
        let receive = crate::receive_stream_when_ready(
            &stream,
            &receive_url,
            self.proof.validate_messaging()?,
            &request,
            |event| {
                if let crate::receive::ReceiveEvent::Record(record) = event {
                    if let Some(push) =
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
        let attempt = async {
            tokio::select! {
                result = &mut receive => { result?; return Err(ProbeError::SessionProtocol(SessionError::UpdateStreamEnded)); }
                ready = ready_rx => { ready.map_err(|_| ProbeError::SessionProtocol(SessionError::UpdateStreamEnded))?; }
            }
            tokio::select! {
                result = &mut receive => { result?; return Err(ProbeError::SessionProtocol(SessionError::UpdateStreamEnded)); }
                result = self.post_request(&short, endpoint, &activation) => { result?; }
            }
            let deadline = tokio::time::sleep(duration);
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    result = &mut receive => { result?; return Err(ProbeError::SessionProtocol(SessionError::UpdateStreamEnded)); }
                    _ = &mut deadline => return Ok(()),
                    push = push_rx.recv() => {
                        let push = push.ok_or(ProbeError::ReceiveFailed)?;
                        let plaintext = self.pairing.pairing.decrypt_payload(&push.ciphertext)
                            .map_err(|_| ProbeError::SessionProtocol(SessionError::UpdateAuthentication))?;
                        let update = crate::updates::decode(self.account_id(), &known,
                            plaintext.as_bytes(), push.request_id.as_str() == session_id)?;
                        if let crate::updates::Update::Conversations(records) = &update {
                            for record in records {
                                if let Some(previous) = known.iter_mut().find(|previous| previous.id == record.id) {
                                    *previous = record.clone();
                                } else {
                                    if known.len() >= 10_000 { return Err(ProbeError::SessionProtocol(SessionError::PaginationLimit)); }
                                    known.push(record.clone());
                                }
                            }
                        }
                        let inactive = matches!(update, crate::updates::Update::Inactive);
                        emit(update)?;
                        if inactive { return Err(ProbeError::ReceiveProtocol(crate::receive::ReceiveError::SessionPreempted)); }
                    }
                }
            }
        };
        tokio::time::timeout(duration + Duration::from_secs(30), attempt)
            .await
            .map_err(|_| ProbeError::Timeout)?
    }

    pub async fn probe_conversations(&self) -> Result<usize, ProbeError> {
        Ok(self.read_conversations().await?.len())
    }

    /// Bounded cursor traversal. This is never an atomic snapshot claim.
    pub async fn read_conversations(
        &self,
    ) -> Result<Vec<handover_core::messaging::Conversation>, ProbeError> {
        tokio::time::timeout(
            Duration::from_secs(120),
            self.probe_startup_at(
                &self.proof.endpoint,
                crate::client(true)?,
                crate::streaming_client()?,
                true,
            ),
        )
        .await
        .map_err(|_| ProbeError::Timeout)??
        .ok_or(ProbeError::UnexpectedResponse)
    }

    async fn acknowledge(
        &self,
        http: &reqwest::Client,
        endpoint: &str,
        id: Zeroizing<String>,
    ) -> Result<(), ProbeError> {
        let mut batch = crate::receive::AckBatch::default();
        batch
            .push(crate::receive::Acknowledgement::processed_session_reply(id))
            .map_err(|_| ProbeError::ReceiveFailed)?;
        let request = self
            .pairing
            .registration
            .acknowledgement_request(&batch)
            .map_err(|_| ProbeError::SessionExpired)?;
        crate::post_acknowledgements(
            http,
            &format!("{endpoint}{}", crate::ACK_MESSAGES_PATH),
            self.proof.validate_messaging()?,
            &request,
        )
        .await
        .map(|_| ())
    }

    async fn post_request(
        &self,
        http: &reqwest::Client,
        endpoint: &str,
        request: &SessionRequest,
    ) -> Result<(), ProbeError> {
        let mut headers = self.proof.validate_messaging()?;
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json+protobuf"),
        );
        let mut response = http
            .post(format!("{endpoint}{}", crate::SEND_MESSAGE_PATH))
            .headers(headers)
            .body(request.body()?.to_vec())
            .send()
            .await
            .map_err(crate::transport_error)?;
        if !response.status().is_success() {
            return Err(crate::http_error_details(response, false).await);
        }
        if response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            != Some("application/json+protobuf")
        {
            return Err(ProbeError::UnexpectedResponse);
        }
        let mut received = 0usize;
        while let Some(chunk) = response.chunk().await.map_err(crate::transport_error)? {
            if chunk.len() > crate::SEND_RESPONSE_LIMIT - received {
                return Err(ProbeError::ResponseTooLarge);
            }
            received += chunk.len();
        }
        Ok(())
    }

    /// Q3a/Qwa activation uses the session ID as request ID, action 16, and an
    /// encrypted field-2 timestamp. The outer transport has no delivery TTL.
    /// Preparing this request changes no remote state.
    pub fn prepare_activation(&self) -> Result<SessionRequest, ProbeError> {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ProbeError::InvalidBootstrap)?
            .as_millis();
        let millis = i64::try_from(millis).map_err(|_| ProbeError::InvalidBootstrap)?;
        let payload = Zeroizing::new(
            ActivationPayload {
                timestamp_millis: millis,
            }
            .encode_to_vec(),
        );
        self.build_request(&self.session_id.to_string(), 16, &payload, 2, None)
    }

    fn build_request(
        &self,
        request_id: &str,
        action: i32,
        payload: &[u8],
        delivery_class: i32,
        ttl: Option<i64>,
    ) -> Result<SessionRequest, ProbeError> {
        // First-party c6 preserves an empty request. It does not manufacture
        // a ciphertext envelope for capability and presence requests.
        let encrypted = if payload.is_empty() {
            Vec::new()
        } else {
            self.pairing
                .encrypt(payload)
                .map_err(|_| ProbeError::InvalidBootstrap)?
                .as_bytes()
                .to_vec()
        };
        let wrapper = EncryptedRequest {
            request_id: request_id.to_owned(),
            action,
            encrypted,
            session_id: self.session_id.to_string(),
        };
        let wrapper = Zeroizing::new(wrapper.encode_to_vec());
        let mut header = self
            .pairing
            .registration
            .messaging_header()
            .map_err(|_| ProbeError::SessionExpired)?;
        header[0] = json!(request_id);
        let mut message = vec![Value::Null; 23];
        message[0] = json!(request_id);
        message[1] = json!(19);
        message[4] = json!(0);
        message[11] = json!(STANDARD.encode(&*wrapper));
        message[22] = json!([null, delivery_class]);
        let mut body = json!([
            [
                16,
                self.proof
                    .account_email
                    .as_deref()
                    .ok_or(ProbeError::InvalidCredentials)?,
                "GDitto"
            ],
            message,
            header,
            null,
            ttl.map(|value| value.to_string()),
            null,
            null,
            null,
            [STANDARD.encode(self.pairing.pairing.peer())]
        ]);
        let bytes = serde_json::to_vec(&body).map_err(|_| ProbeError::InvalidBootstrap);
        crate::registration::erase_strings(&mut body);
        Ok(SessionRequest {
            bytes: Zeroizing::new(bytes?),
            started: Instant::now(),
            lifetime: self
                .pairing
                .registration
                .remaining_lifetime()
                .map_err(|_| ProbeError::SessionExpired)?,
        })
    }
}

pub struct SessionRequest {
    bytes: Zeroizing<Vec<u8>>,
    started: Instant,
    lifetime: Duration,
}
impl fmt::Debug for SessionRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionRequest { redacted }")
    }
}
impl SessionRequest {
    pub(crate) fn body(&self) -> Result<&[u8], ProbeError> {
        if self.started.elapsed() >= self.lifetime {
            return Err(ProbeError::SessionExpired);
        }
        Ok(&self.bytes)
    }
}
#[derive(Message)]
struct ActivationPayload {
    #[prost(int64, tag = "2")]
    timestamp_millis: i64,
}
#[derive(Message)]
struct EncryptedRequest {
    #[prost(string, tag = "1")]
    request_id: String,
    #[prost(int32, tag = "2")]
    action: i32,
    #[prost(bytes = "vec", tag = "5")]
    encrypted: Vec<u8>,
    #[prost(string, tag = "6")]
    session_id: String,
}
impl Drop for EncryptedRequest {
    fn drop(&mut self) {
        self.request_id.zeroize();
        self.session_id.zeroize();
        self.encrypted.zeroize();
    }
}

#[derive(Message)]
struct ConversationRequest {
    #[prost(int32, tag = "2")]
    limit: i32,
    #[prost(int32, tag = "4")]
    status: i32,
    #[prost(message, optional, tag = "5")]
    cursor: Option<ConversationCursor>,
}
#[derive(Message)]
struct ConversationPage {
    #[prost(bytes = "vec", repeated, tag = "2")]
    conversations: Vec<Vec<u8>>,
    #[prost(message, optional, tag = "5")]
    cursor: Option<ConversationCursor>,
}
impl Drop for ConversationPage {
    fn drop(&mut self) {
        self.conversations.iter_mut().for_each(Zeroize::zeroize);
    }
}
#[derive(Message)]
struct ConversationCursor {
    #[prost(string, tag = "1")]
    id: String,
    #[prost(int64, tag = "2")]
    timestamp: i64,
}
impl Drop for ConversationCursor {
    fn drop(&mut self) {
        self.id.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use handover_core::messaging::{
        Conversation, ConversationId, ConversationKind, MessagingAccountId, Participant,
        TransportKind,
    };

    #[test]
    fn repeated_page_entries_merge_only_when_the_normalized_record_agrees() {
        let record = Conversation {
            id: ConversationId::new(MessagingAccountId::new("fixture"), "thread"),
            kind: ConversationKind::Direct,
            transport: TransportKind::Unknown,
            title: None,
            participants: vec![Participant {
                local_id: "peer".into(),
                display_name: None,
                address: None,
                is_self: false,
            }],
            latest_message_id: None,
            last_activity_at: None,
            unread_count: None,
            cursor: None,
            capabilities: Default::default(),
        };
        assert_eq!(
            unique_conversations(vec![record.clone(), record.clone()]).unwrap(),
            vec![record.clone()]
        );
        let mut conflict = record.clone();
        conflict.unread_count = Some(1);
        assert!(matches!(
            unique_conversations(vec![record, conflict]),
            Err(ProbeError::SessionProtocol(
                SessionError::DuplicateConversations
            ))
        ));
    }
}
