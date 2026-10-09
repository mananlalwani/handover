//! Separate first-party RCS receive connection. Protocol material stays private.
mod composing;

use crate::{
    ProbeError,
    receive::ReceiveError,
    session::{LiveEvent, LiveOutput},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use handover_core::messaging::{Conversation, ConversationKind, TransportKind};
use p256::pkcs8::EncodePublicKey;
use prost::Message;
use reqwest::header::HeaderMap;
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};
use tokio::sync::{mpsc, watch};
use zeroize::{Zeroize, Zeroizing};

const SIGN_IN: &str =
    "/$rpc/google.internal.communications.instantmessaging.v1.Registration/SignInSecondary";

#[derive(Clone, PartialEq, Eq)]
struct Config {
    identity: Zeroizing<String>,
    endpoint: String,
}

pub(crate) struct Context {
    settings: watch::Sender<Option<Config>>,
    conversations: watch::Sender<Vec<Conversation>>,
}
impl Default for Context {
    fn default() -> Self {
        Self {
            settings: watch::channel(None).0,
            conversations: watch::channel(Vec::new()).0,
        }
    }
}
impl Context {
    /// Only call with an authenticated, decrypted settings push from the pinned phone.
    pub(crate) fn settings(&self, bytes: &[u8]) {
        let config = settings(bytes).unwrap_or_default();
        {
            self.settings.send_if_modified(|previous| {
                if *previous == config {
                    false
                } else {
                    *previous = config;
                    true
                }
            });
        }
    }
    pub(crate) fn conversations(&self, records: &[Conversation]) {
        self.conversations.send_replace(records.to_vec());
    }
    pub(crate) async fn run(
        &self,
        proof: &crate::BrowserProof,
        device: &str,
        events: mpsc::Sender<LiveOutput>,
    ) {
        let Some(email) = proof
            .account_email
            .as_deref()
            .filter(|value| crate::valid_account_email(value))
        else {
            std::future::pending::<()>().await;
            return;
        };
        let Ok(headers) = proof.validate_messaging() else {
            return;
        };
        let Ok(http) = crate::client(true) else {
            return;
        };
        let Ok(stream) = crate::streaming_client() else {
            return;
        };
        let transport = Transport {
            http,
            stream,
            headers,
        };
        let mut config = self.settings.subscribe();
        let known = self.conversations.subscribe();
        // The first-party client reuses its persisted web device across applications.
        // Reconnects must not accumulate new server registrations.

        let key = p256::SecretKey::random(&mut rand::rngs::OsRng);
        let mut delay = 1;
        let mut active = BTreeMap::new();
        loop {
            let current = config.borrow_and_update().clone();
            if let Some(current) = current {
                let attempt = async {
                    let registration = register(&transport, &current, email, device, &key).await?;
                    eprintln!("native RCS registered");
                    receive(
                        &transport,
                        &current,
                        &registration,
                        known.clone(),
                        &events,
                        &mut active,
                    )
                    .await
                };
                let (changed, terminal) = tokio::select! {
                    biased;
                    changed = config.changed() => { if changed.is_err() { return; } (true, false) }
                    result = attempt => {
                        let terminal = result.as_ref().err().is_some_and(|error| {
                            matches!(error.http_status(), Some(400 | 401 | 403))
                                || matches!(error.rpc_status(), Some(crate::RpcStatus::Unauthenticated | crate::RpcStatus::PermissionDenied | crate::RpcStatus::InvalidArgument))
                        });
                        if let Err(error) = result { eprintln!("native RCS receive stopped: {error:?}"); }
                        (false, terminal)
                    }
                };
                // Cancellation of a changed registration also clears published indicators.
                for (conversation, _) in std::mem::take(&mut active) {
                    if publish(&events, conversation, Vec::new()).await.is_err() {
                        return;
                    }
                }
                if changed {
                    delay = 1;
                    continue;
                }
                if terminal {
                    // Authentication and malformed registration require new settings or a new owner.
                    if config.changed().await.is_err() {
                        return;
                    }
                    delay = 1;
                    continue;
                }
                tokio::select! {
                    changed = config.changed() => { if changed.is_err() { return; } delay = 1; }
                    _ = tokio::time::sleep(Duration::from_secs(delay)) => { delay = (delay * 2).min(60); }
                }
            } else if config.changed().await.is_err() {
                return;
            }
        }
    }
}

fn invalid() -> ProbeError {
    ProbeError::UnexpectedResponse
}
fn endpoint(value: &str) -> Result<String, ProbeError> {
    if value.len() > 512 {
        return Err(ProbeError::InvalidEndpoint);
    }
    let value = if value.starts_with("https://") {
        value.to_owned()
    } else {
        format!("https://{value}")
    };
    let url = reqwest::Url::parse(&value).map_err(|_| ProbeError::InvalidEndpoint)?;
    let host = url.host_str().ok_or(ProbeError::InvalidEndpoint)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !(host.ends_with(".googleapis.com") || host.ends_with(".clients6.google.com"))
    {
        return Err(ProbeError::InvalidEndpoint);
    }
    // Same first-party service mapping as the web client; never accept arbitrary URLs.
    let host = host
        .strip_suffix(".googleapis.com")
        .map(|prefix| format!("{prefix}.clients6.google.com"))
        .unwrap_or_else(|| host.to_owned());
    let host = host.replace("-preprod", "");
    Ok(format!("https://{host}"))
}

#[derive(Message)]
#[prost(skip_debug)]
struct SettingsUpdate {
    #[prost(message, optional, tag = "5")]
    settings: Option<Settings>,
}
#[derive(Message)]
#[prost(skip_debug)]
struct Settings {
    #[prost(message, repeated, tag = "2")]
    sims: Vec<Sim>,
}
#[derive(Message)]
#[prost(skip_debug)]
struct Sim {
    #[prost(message, optional, tag = "5")]
    identity: Option<SimIdentity>,
    #[prost(string, tag = "9")]
    endpoint: String,
}
#[derive(Message)]
#[prost(skip_debug)]
struct SimIdentity {
    #[prost(string, tag = "7")]
    address: String,
}
impl Drop for SimIdentity {
    fn drop(&mut self) {
        self.address.zeroize();
    }
}
fn settings(bytes: &[u8]) -> Result<Option<Config>, ProbeError> {
    if bytes.len() > 512 * 1024 {
        return Err(invalid());
    }
    let update = SettingsUpdate::decode(bytes).map_err(|_| invalid())?;
    let Some(settings) = update.settings else {
        return Err(invalid());
    };
    if settings.sims.len() > 16 {
        return Err(invalid());
    }
    let mut result = None;
    for sim in settings.sims {
        if sim.endpoint.is_empty() {
            continue;
        }
        let identity = sim.identity.ok_or_else(invalid)?;
        if identity.address.is_empty()
            || identity.address.len() > 256
            || identity.address.chars().any(char::is_control)
        {
            return Err(invalid());
        }
        let config = Config {
            identity: Zeroizing::new(identity.address.clone()),
            endpoint: endpoint(&sim.endpoint)?,
        };
        // Do not guess which SIM owns a conversation when the phone has several RCS identities.
        if result.as_ref().is_some_and(|previous| previous != &config) {
            return Err(invalid());
        }
        result = Some(config);
    }
    Ok(result)
}

struct Transport {
    http: reqwest::Client,
    stream: reqwest::Client,
    headers: HeaderMap,
}

struct Registration {
    token: Zeroizing<Vec<u8>>,
    lifetime: Duration,
}
fn header(token: Option<&[u8]>) -> Value {
    let mut header = crate::request_header();
    header[2] = json!("RCS");
    if let Some(token) = token {
        header[5] = json!(STANDARD.encode(token));
    }
    header
}
async fn register(
    transport: &Transport,
    config: &Config,
    email: &str,
    device: &str,
    key: &p256::SecretKey,
) -> Result<Registration, ProbeError> {
    let public = key
        .public_key()
        .to_public_key_der()
        .map_err(|_| ProbeError::NativeError)?;
    let mut info = vec![Value::Null; 30];
    info[0] = json!([3, device]);
    info[5] = json!([2, STANDARD.encode(public.as_bytes())]);
    info[7] = json!("en-US");
    // RCS application declaration and receive features from SignInSecondary.
    info[15] = json!([[6]]);
    info[29] = json!([[4, 5]]);
    let mut request = json!([
        header(None),
        [16, email, "RCS"],
        [1, config.identity.as_str(), "RCS"],
        info
    ]);
    let response = crate::query_response(
        &transport.http,
        &format!("{}{SIGN_IN}", config.endpoint),
        transport.headers.clone(),
        &request,
        false,
    )
    .await;
    crate::registration::erase_strings(&mut request);
    let response = response?;
    let mut body: Value = serde_json::from_slice(&response).map_err(|_| invalid())?;
    let result = (|| {
        let auth = body.get(1).and_then(Value::as_array).ok_or_else(invalid)?;
        let token = auth.first().and_then(Value::as_array).ok_or_else(invalid)?;
        let encoded = token
            .first()
            .and_then(Value::as_str)
            .filter(|value| value.len() <= 11000)
            .ok_or_else(invalid)?;
        let token_bytes = Zeroizing::new(STANDARD.decode(encoded).map_err(|_| invalid())?);
        if token_bytes.is_empty() || token_bytes.len() > 8192 {
            return Err(invalid());
        }
        let micros = token
            .get(1)
            .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
            .filter(|value| *value > 0)
            .ok_or_else(invalid)?;
        Ok(Registration {
            token: token_bytes,
            lifetime: Duration::from_micros(micros).min(Duration::from_secs(86400)),
        })
    })();
    crate::registration::erase_strings(&mut body);
    result
}

struct TypingEvent {
    inbox: Zeroizing<String>,
    sender: Zeroizing<String>,
    composing: composing::Composing,
    timestamp_micros: u64,
}
#[derive(Message)]
#[prost(skip_debug)]
struct ContentBatch {
    #[prost(message, repeated, tag = "1")]
    contents: Vec<Content>,
}
#[derive(Message)]
#[prost(skip_debug)]
struct Content {
    #[prost(string, tag = "1")]
    mime: String,
    #[prost(bytes = "vec", tag = "2")]
    data: Vec<u8>,
}
impl Drop for Content {
    fn drop(&mut self) {
        self.data.zeroize();
    }
}
fn typing(record: &crate::receive::ReceiveRecord) -> Result<Option<TypingEvent>, ReceiveError> {
    let fields = record.fields();
    if fields.len() > 7 {
        return Err(ReceiveError::InvalidEvents);
    }
    let Some(message) = fields.get(1).and_then(Value::as_array) else {
        return Ok(None);
    };
    if message.len() > 128 {
        return Err(ReceiveError::InvalidEnvelope);
    }
    if fields.iter().skip(2).any(|value| !value.is_null()) {
        return Err(ReceiveError::InvalidEvents);
    }
    if message.get(1).and_then(Value::as_u64) != Some(36) {
        return Ok(None);
    }
    // Group identity binding and encrypted content require their own attested mappings.
    if message.get(9).is_some_and(|value| !value.is_null()) {
        return Ok(None);
    }
    let Some(sender) = message.get(7).and_then(Value::as_array) else {
        return Err(ReceiveError::InvalidIdentifiers);
    };
    if sender.first().and_then(Value::as_u64) != Some(1)
        || sender.get(2).and_then(Value::as_str) != Some("RCS")
    {
        return Ok(None);
    }
    let sender = sender
        .get(1)
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
        })
        .ok_or(ReceiveError::InvalidIdentifiers)?;
    let id = message
        .first()
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
        })
        .ok_or(ReceiveError::InvalidIdentifiers)?;
    let encoded = message
        .get(11)
        .and_then(Value::as_str)
        .ok_or(ReceiveError::InvalidEnvelope)?;
    // Large content is outside composing support, not a reason to drop this stream.
    if encoded.len() > 24000 {
        return Ok(None);
    }
    let content = Zeroizing::new(
        STANDARD
            .decode(encoded)
            .map_err(|_| ReceiveError::InvalidEnvelope)?,
    );
    let batch =
        ContentBatch::decode(content.as_slice()).map_err(|_| ReceiveError::InvalidEnvelope)?;
    if batch.contents.len() != 1 {
        return Ok(None);
    }
    let content = &batch.contents[0];
    if !content.mime.split(';').next().is_some_and(|value| {
        value
            .trim()
            .eq_ignore_ascii_case("application/im-iscomposing+xml")
    }) {
        return Ok(None);
    }
    let timestamp_micros = message
        .get(2)
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
        .filter(|value| *value > 0)
        .ok_or(ReceiveError::InvalidEnvelope)?;
    let composing = composing::decode(&content.data).map_err(|_| ReceiveError::InvalidEnvelope)?;
    Ok(Some(TypingEvent {
        inbox: Zeroizing::new(id.to_owned()),
        sender: Zeroizing::new(sender.to_owned()),
        composing,
        timestamp_micros,
    }))
}

fn bind<'a>(known: &'a [Conversation], sender: &str) -> Option<(&'a str, &'a str)> {
    let mut matches = known
        .iter()
        .filter(|conversation| {
            conversation.kind == ConversationKind::Direct
                && conversation.transport == TransportKind::Rcs
        })
        .filter_map(|conversation| {
            let mut peers = conversation
                .participants
                .iter()
                .filter(|participant| !participant.is_self);
            let peer = peers.next()?;
            if peers.next().is_some() || peer.address.as_deref() != Some(sender) {
                return None;
            }
            Some((conversation.id.local_id.as_str(), peer.local_id.as_str()))
        });
    let found = matches.next()?;
    if matches.next().is_some() {
        None
    } else {
        Some(found)
    }
}

fn fresh(event: &TypingEvent, now: Duration, previous: Option<u64>) -> Option<Duration> {
    if previous.is_some_and(|previous| previous >= event.timestamp_micros) {
        return None;
    }
    let timestamp = Duration::from_micros(event.timestamp_micros);
    let lifetime = match event.composing {
        composing::Composing::Active(refresh) => refresh,
        composing::Composing::Idle => Duration::from_secs(120),
    };
    let age = now.saturating_sub(timestamp);
    if timestamp > now + Duration::from_secs(5) || age >= lifetime {
        return None;
    }
    Some(lifetime - age)
}

async fn receive(
    transport: &Transport,
    config: &Config,
    registration: &Registration,
    known: watch::Receiver<Vec<Conversation>>,
    events: &mpsc::Sender<LiveOutput>,
    active: &mut BTreeMap<String, (String, tokio::time::Instant)>,
) -> Result<(), ProbeError> {
    let mut request = json!([header(Some(&registration.token)), null, null, []]);
    let body = Zeroizing::new(serde_json::to_vec(&request).map_err(|_| invalid())?);
    crate::registration::erase_strings(&mut request);
    let (tx, mut rx) = mpsc::channel(16);
    let url = format!("{}{}", config.endpoint, crate::RECEIVE_MESSAGES_PATH);
    let receive = crate::receive_json_when_ready(
        &transport.stream,
        &url,
        transport.headers.clone(),
        &body,
        |event| {
            if let crate::receive::ReceiveEvent::Record(record) = event {
                if let Some(typing) = typing(&record)? {
                    tx.try_send(typing)
                        .map_err(|_| ReceiveError::Backpressure)?;
                }
            }
            Ok(())
        },
        || {},
    );
    tokio::pin!(receive);
    let expiry = tokio::time::sleep(registration.lifetime);
    tokio::pin!(expiry);
    let mut latest = BTreeMap::<String, u64>::new();
    async {
        loop {
            let next_expiry = active.values().map(|(_, expires)| *expires).min().unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(86400));
            tokio::select! {
                result = &mut receive => { result?; return Err(ProbeError::ReceiveFailed); }
                _ = &mut expiry => return Err(ProbeError::SessionExpired),
                _ = tokio::time::sleep_until(next_expiry) => {
                    let expired: Vec<_> = active.iter().filter(|(_, (_, expires))| *expires <= tokio::time::Instant::now()).map(|(id, _)| id.clone()).collect();
                    for conversation in expired { active.remove(&conversation); publish(events, conversation, Vec::new()).await?; }
                }
                event = rx.recv() => {
                    let event = event.ok_or(ProbeError::ReceiveFailed)?;
                    let binding = bind(&known.borrow(), &event.sender).map(|(conversation, participant)| (conversation.to_owned(), participant.to_owned()));
                    let Some((conversation, participant)) = binding else { continue; };
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|_| ProbeError::NativeError)?;
                    if let Some(remaining) = fresh(&event, now, latest.get(&conversation).copied()) {
                        if latest.len() >= 256 && !latest.contains_key(&conversation) { return Err(ProbeError::ReceiveProtocol(ReceiveError::Backpressure)); }
                        latest.insert(conversation.clone(), event.timestamp_micros);
                        let participants = match event.composing {
                            composing::Composing::Active(_) => {
                                active.insert(conversation.clone(), (participant.clone(), tokio::time::Instant::now() + remaining));
                                vec![participant]
                            }
                            composing::Composing::Idle => { active.remove(&conversation); Vec::new() }
                        };
                        publish(events, conversation, participants).await?;
                    }
                    let mut ack = json!([header(Some(&registration.token)), [event.inbox.as_str()]]);
                    let result = crate::query_response(&transport.http, &format!("{}{}", config.endpoint, crate::ACK_MESSAGES_PATH), transport.headers.clone(), &ack, false).await;
                    crate::registration::erase_strings(&mut ack);
                    result?;
                }
            }
        }
    }.await
}
async fn publish(
    events: &mpsc::Sender<LiveOutput>,
    conversation: String,
    participants: Vec<String>,
) -> Result<(), ProbeError> {
    let (accepted, processed) = tokio::sync::oneshot::channel();
    events
        .send(LiveOutput {
            event: LiveEvent::Typing {
                conversation,
                participants,
            },
            accepted,
        })
        .await
        .map_err(|_| ProbeError::NativeError)?;
    tokio::time::timeout(Duration::from_secs(30), processed)
        .await
        .map_err(|_| ProbeError::Timeout)?
        .map_err(|_| ProbeError::NativeError)
}

#[cfg(test)]
mod tests;
