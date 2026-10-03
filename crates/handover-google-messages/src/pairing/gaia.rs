//! Offline Gaia pairing envelopes and correlated phone confirmation.
//! Transport and emoji display remain absent; no type creates an online account.

use prost::Message;
use std::{
    fmt,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use super::{HandshakeError, PairingHandshake, PendingPhoneConfirmation};
use crate::sources::RegisteredPhone;

const RESPONSE_LIMIT: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingError {
    IneligiblePhone,
    Clock,
    Correlation,
    InvalidResponse,
    Rejected,
    ConfirmationRequired,
    UnsupportedRevision,
    InvalidRouting,
    Expired,
    Handshake(HandshakeError),
}

impl From<HandshakeError> for PairingError {
    fn from(value: HandshakeError) -> Self {
        Self::Handshake(value)
    }
}

/// One selected peer, pairing ID, and transport request ID. No network effects.
pub struct InitialPairing {
    handshake: PairingHandshake,
    peer: Vec<u8>,
    pairing_id: String,
    request_id: String,
    timestamp_millis: i64,
    request: Vec<u8>,
}

impl fmt::Debug for InitialPairing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InitialPairing { redacted }")
    }
}

impl InitialPairing {
    pub fn prepare(phone: &RegisteredPhone) -> Result<Self, PairingError> {
        let peer = phone
            .pairing_identity()
            .ok_or(PairingError::IneligiblePhone)?
            .to_vec();
        let timestamp_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| PairingError::Clock)?
            .as_millis()
            .try_into()
            .map_err(|_| PairingError::Clock)?;
        let handshake = PairingHandshake::start();
        let pairing_id = Uuid::new_v4().to_string();
        let request_id = Uuid::new_v4().to_string();
        let request = envelope(
            &pairing_id,
            timestamp_millis,
            handshake.client_init()?,
            true,
        );
        Ok(Self {
            handshake,
            peer,
            pairing_id,
            request_id,
            timestamp_millis,
            request,
        })
    }

    pub(crate) fn peer(&self) -> &[u8] {
        &self.peer
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Inner request for message type 44. An eventual transport must supply the
    /// separate routing envelope and check registration expiry before sending.
    pub fn request_bytes(&self) -> Result<&[u8], PairingError> {
        self.handshake.client_init()?;
        Ok(&self.request)
    }

    /// Build the observed outer type-44 routing envelope. It remains an
    /// unsigned protobuf payload until the authenticated Tachyon transport adds
    /// its current request header.
    pub(crate) fn send_envelope_with_lifetime(
        &self,
        account_id: &str,
        session_id: &str,
        registration_token: &[u8],
        lifetime: Duration,
    ) -> Result<PairingSendEnvelope, PairingError> {
        self.handshake.client_init()?;
        let lifetime = lifetime.min(self.handshake.remaining_lifetime()?);
        PairingSendEnvelope::build(
            account_id,
            session_id,
            PairingSendDetails {
                request_id: &self.request_id,
                peer: &self.peer,
                registration_token,
                request_type: 44,
                request: &self.request,
                lifetime,
            },
        )
    }

    /// The eventual receive dispatcher supplies the outer request correlation
    /// and sender identity. Both are checked before decoding the inner response.
    /// Consumes the attempt on success or failure; it never retries.
    pub fn accept_response(
        self,
        request_id: &str,
        sender: &[u8],
        body: &[u8],
    ) -> Result<AwaitingPhoneConfirmation, PairingError> {
        self.handshake.client_init()?;
        if request_id != self.request_id || sender != self.peer {
            return Err(PairingError::Correlation);
        }
        if body.is_empty() || body.len() > RESPONSE_LIMIT {
            return Err(PairingError::InvalidResponse);
        }
        let response = InitialResponse::decode(body).map_err(|_| PairingError::InvalidResponse)?;
        if response.pairing_id != self.pairing_id {
            return Err(PairingError::Correlation);
        }
        if response.status != 0 {
            return Err(PairingError::Rejected);
        }
        if !response.confirmation_required {
            return Err(PairingError::ConfirmationRequired);
        }
        // The browser also supports revision 0. Its derivation/display contract
        // has not been implemented here, so do not silently fall back to it.
        if response.auth_revision != 1
            || response.key_revision != 1
            || response.feature_revision != 0
        {
            return Err(PairingError::UnsupportedRevision);
        }
        let server = response.handshake.ok_or(PairingError::InvalidResponse)?;
        let pending = self.handshake.accept_server_init(&server)?;
        let request = envelope(
            &self.pairing_id,
            self.timestamp_millis,
            pending.client_finish()?,
            false,
        );
        Ok(AwaitingPhoneConfirmation {
            pending,
            peer: self.peer,
            pairing_id: self.pairing_id,
            request_id: Uuid::new_v4().to_string(),
            request,
        })
    }
}

/// Valid initial response only. Phone confirmation has not completed.
/// Session keys remain private until the full channel contract is implemented.
pub struct AwaitingPhoneConfirmation {
    pending: PendingPhoneConfirmation,
    peer: Vec<u8>,
    pairing_id: String,
    request_id: String,
    request: Vec<u8>,
}

impl fmt::Debug for AwaitingPhoneConfirmation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AwaitingPhoneConfirmation { redacted }")
    }
}

impl AwaitingPhoneConfirmation {
    /// The observed revision-1 symbol for display while the phone asks for a
    /// match. Producing a symbol does not confirm that the user selected it.
    pub fn verification_emoji(
        &self,
    ) -> Result<super::verification::VerificationEmoji, PairingError> {
        Ok(super::verification::revision_one(
            self.pending.auth_string()?,
        ))
    }

    /// Raw verification bytes. Human-readable emoji mapping remains unfinished.
    pub fn auth_string(&self) -> Result<&[u8; 32], PairingError> {
        Ok(self.pending.auth_string()?)
    }

    pub(crate) fn peer(&self) -> &[u8] {
        &self.peer
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Inner client-finish request for message type 45, using the original
    /// pairing ID and timestamp but a fresh outer request ID.
    pub fn request_bytes(&self) -> Result<&[u8], PairingError> {
        self.pending.client_finish()?;
        Ok(&self.request)
    }

    /// Build the observed outer type-45 routing envelope. It remains an
    /// unsigned protobuf payload until the authenticated Tachyon transport adds
    /// its current request header.
    pub(crate) fn send_envelope_with_lifetime(
        &self,
        account_id: &str,
        session_id: &str,
        registration_token: &[u8],
        lifetime: Duration,
    ) -> Result<PairingSendEnvelope, PairingError> {
        self.pending.client_finish()?;
        let lifetime = lifetime.min(self.pending.remaining_lifetime()?);
        PairingSendEnvelope::build(
            account_id,
            session_id,
            PairingSendDetails {
                request_id: &self.request_id,
                peer: &self.peer,
                registration_token,
                request_type: 45,
                request: &self.request,
                lifetime,
            },
        )
    }

    /// A correlated phone response confirms only this pairing exchange. It
    /// does not prove a working receive channel or restore a messaging account.
    pub fn accept_response(
        self,
        request_id: &str,
        sender: &[u8],
        body: &[u8],
    ) -> Result<PhoneConfirmedPairing, PairingError> {
        self.pending.client_finish()?;
        if request_id != self.request_id || sender != self.peer {
            return Err(PairingError::Correlation);
        }
        if body.is_empty() || body.len() > RESPONSE_LIMIT {
            return Err(PairingError::InvalidResponse);
        }
        let mut response =
            FinalResponse::decode(body).map_err(|_| PairingError::InvalidResponse)?;
        if response.pairing_id != self.pairing_id {
            return Err(PairingError::Correlation);
        }
        if response.status != 0 {
            return Err(PairingError::Rejected);
        }
        if response.encrypted_user_data.len() > 8192 {
            return Err(PairingError::InvalidResponse);
        }
        Ok(PhoneConfirmedPairing {
            keys: super::keys::GaiaKeys::derive(&self.pending._next_protocol_secret)?,
            _peer: self.peer,
            _pairing_id: self.pairing_id,
            _encrypted_user_data: Zeroizing::new(std::mem::take(&mut response.encrypted_user_data)),
        })
    }
}

/// Redacted protobuf payload for Messaging/SendMessage. It is not authenticated
/// or sent, and callers must not treat construction as pairing progress.
pub struct PairingSendEnvelope {
    bytes: Zeroizing<Vec<u8>>,
    started: Instant,
    lifetime: Duration,
}

impl fmt::Debug for PairingSendEnvelope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PairingSendEnvelope { redacted }")
    }
}

impl PairingSendEnvelope {
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    pub(crate) fn ensure_valid(&self) -> Result<(), PairingError> {
        if self.started.elapsed() >= self.lifetime {
            return Err(PairingError::Expired);
        }
        Ok(())
    }

    /// JSON-protobuf outer transport used by Google's Messaging client. The
    /// embedded Ditto wrapper and UKEY2 payload remain binary base64 fields.
    pub(crate) fn json_request(&self) -> Result<Zeroizing<Vec<u8>>, PairingError> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        use serde_json::{Value, json};

        self.ensure_valid()?;
        let outer = MessagingSend::decode(self.bytes.as_slice())
            .map_err(|_| PairingError::InvalidRouting)?;
        let destination = outer
            .destination
            .as_ref()
            .ok_or(PairingError::InvalidRouting)?;
        let message = outer.message.as_ref().ok_or(PairingError::InvalidRouting)?;
        let routing = message
            .routing
            .as_ref()
            .ok_or(PairingError::InvalidRouting)?;
        let header = outer
            .authentication
            .as_ref()
            .ok_or(PairingError::InvalidRouting)?;
        let client = header
            .client_info
            .as_ref()
            .ok_or(PairingError::InvalidRouting)?;
        let mut wire_message = vec![Value::Null; 23];
        wire_message[0] = json!(message.request_id);
        wire_message[1] = json!(message.kind);
        wire_message[11] = json!(STANDARD.encode(&message.payload));
        wire_message[22] = json!([null, routing.delivery_class]);
        let mut body = json!([
            [destination.kind, destination.id, destination.client],
            wire_message,
            [
                header.request_id,
                null,
                header.application,
                null,
                null,
                STANDARD.encode(&header.registration_token),
                [
                    null,
                    null,
                    client.wire_year,
                    client.wire_major,
                    client.wire_minor,
                    null,
                    client.client_type,
                    null,
                    client.platform_type
                ]
            ],
            null,
            outer.maximum_lifetime_micros.to_string(),
            null,
            null,
            null,
            outer
                .recipient_identities
                .iter()
                .map(|id| STANDARD.encode(id))
                .collect::<Vec<_>>()
        ]);
        let encoded = serde_json::to_vec(&body)
            .map(Zeroizing::new)
            .map_err(|_| PairingError::InvalidRouting);
        fn erase_strings(value: &mut Value) {
            match value {
                Value::String(text) => text.zeroize(),
                Value::Array(values) => values.iter_mut().for_each(erase_strings),
                _ => {}
            }
        }
        erase_strings(&mut body);
        encoded
    }

    fn build(
        account_id: &str,
        session_id: &str,
        details: PairingSendDetails<'_>,
    ) -> Result<Self, PairingError> {
        let PairingSendDetails {
            request_id,
            peer,
            registration_token,
            request_type,
            request,
            lifetime,
        } = details;
        if account_id.is_empty()
            || account_id.len() > 320
            || account_id.chars().any(char::is_control)
            || (!session_id.is_empty() && Uuid::parse_str(session_id).is_err())
            || Uuid::parse_str(request_id).is_err()
            || peer.is_empty()
            || peer.len() > 1024
            || registration_token.is_empty()
            || registration_token.len() > 8192
            || request.is_empty()
            || request.len() > RESPONSE_LIMIT
            || lifetime.is_zero()
        {
            return Err(PairingError::InvalidRouting);
        }
        let wrapper = PairingRequestWrapper {
            request_id: request_id.to_owned(),
            request_type,
            request: request.to_vec(),
            session_id: session_id.to_owned(),
        }
        .encode_to_vec();
        let vc = SendMessage {
            request_id: request_id.to_owned(),
            kind: 19,
            payload: wrapper,
            routing: Some(RoutingMetadata {
                delivery_class: if request_type == 44 { 20 } else { 2 },
            }),
        };
        let outer = MessagingSend {
            destination: Some(AccountDestination {
                kind: 16,
                id: account_id.to_owned(),
                client: "GDitto".to_owned(),
            }),
            message: Some(vc),
            maximum_lifetime_micros: 300_000_000,
            recipient_identities: vec![peer.to_vec()],
            authentication: Some(TachyonRequestHeader {
                request_id: request_id.to_owned(),
                application: "GDitto".to_owned(),
                registration_token: registration_token.to_vec(),
                client_info: Some(TachyonClientInfo {
                    wire_year: 20261001,
                    wire_major: 2,
                    wire_minor: 0,
                    client_type: 4,
                    platform_type: 6,
                }),
            }),
        };
        Ok(Self {
            bytes: Zeroizing::new(outer.encode_to_vec()),
            started: Instant::now(),
            lifetime,
        })
    }
}

struct PairingSendDetails<'a> {
    request_id: &'a str,
    peer: &'a [u8],
    registration_token: &'a [u8],
    request_type: i32,
    request: &'a [u8],
    lifetime: Duration,
}

#[derive(Message)]
struct AccountDestination {
    #[prost(int32, tag = "1")]
    kind: i32,
    #[prost(string, tag = "2")]
    id: String,
    #[prost(string, tag = "3")]
    client: String,
}

impl Drop for AccountDestination {
    fn drop(&mut self) {
        self.id.zeroize();
    }
}

#[derive(Message)]
struct RoutingMetadata {
    #[prost(int32, tag = "2")]
    delivery_class: i32,
}

#[derive(Message)]
struct PairingRequestWrapper {
    #[prost(string, tag = "1")]
    request_id: String,
    #[prost(int32, tag = "2")]
    request_type: i32,
    #[prost(bytes = "vec", tag = "3")]
    request: Vec<u8>,
    #[prost(string, tag = "6")]
    session_id: String,
}

impl Drop for PairingRequestWrapper {
    fn drop(&mut self) {
        self.request.zeroize();
    }
}

#[derive(Message)]
struct SendMessage {
    #[prost(string, tag = "1")]
    request_id: String,
    #[prost(int32, tag = "2")]
    kind: i32,
    #[prost(bytes = "vec", tag = "12")]
    payload: Vec<u8>,
    #[prost(message, optional, tag = "23")]
    routing: Option<RoutingMetadata>,
}

impl Drop for SendMessage {
    fn drop(&mut self) {
        self.payload.zeroize();
    }
}

#[derive(Message)]
struct MessagingSend {
    #[prost(message, optional, tag = "1")]
    destination: Option<AccountDestination>,
    #[prost(message, optional, tag = "2")]
    message: Option<SendMessage>,
    #[prost(message, optional, tag = "3")]
    authentication: Option<TachyonRequestHeader>,
    #[prost(int64, tag = "5")]
    maximum_lifetime_micros: i64,
    #[prost(bytes = "vec", repeated, tag = "9")]
    recipient_identities: Vec<Vec<u8>>,
}

impl Drop for MessagingSend {
    fn drop(&mut self) {
        self.recipient_identities
            .iter_mut()
            .for_each(Zeroize::zeroize);
    }
}

#[derive(Message)]
struct TachyonRequestHeader {
    #[prost(string, tag = "1")]
    request_id: String,
    #[prost(string, tag = "3")]
    application: String,
    #[prost(bytes = "vec", tag = "6")]
    registration_token: Vec<u8>,
    #[prost(message, optional, tag = "7")]
    client_info: Option<TachyonClientInfo>,
}

impl Drop for TachyonRequestHeader {
    fn drop(&mut self) {
        self.registration_token.zeroize();
    }
}

#[derive(Message)]
struct TachyonClientInfo {
    #[prost(int32, tag = "3")]
    wire_year: i32,
    #[prost(int32, tag = "4")]
    wire_major: i32,
    #[prost(int32, tag = "5")]
    wire_minor: i32,
    #[prost(int32, tag = "7")]
    client_type: i32,
    #[prost(int32, tag = "9")]
    platform_type: i32,
}

/// Acknowledged phone exchange, with no online state, persistence, or key export.
pub struct PhoneConfirmedPairing {
    keys: super::keys::GaiaKeys,
    _peer: Vec<u8>,
    _pairing_id: String,
    _encrypted_user_data: Zeroizing<Vec<u8>>,
}

impl fmt::Debug for PhoneConfirmedPairing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PhoneConfirmedPairing { redacted }")
    }
}

impl PhoneConfirmedPairing {
    pub(crate) fn stored_record(&self) -> Zeroizing<Vec<u8>> {
        let (first, second) = self.keys.stored_keys();
        Zeroizing::new(
            StoredPairing {
                version: 1,
                first: first.to_vec(),
                second: second.to_vec(),
                peer: self._peer.clone(),
                pairing_id: self._pairing_id.clone(),
                encrypted_user_data: self._encrypted_user_data.to_vec(),
            }
            .encode_to_vec(),
        )
    }

    pub(crate) fn restore_record(record: &[u8]) -> Result<Self, PairingError> {
        let mut stored =
            StoredPairing::decode(record).map_err(|_| PairingError::InvalidResponse)?;
        if stored.version != 1
            || stored.peer.is_empty()
            || stored.peer.len() > 1024
            || Uuid::parse_str(&stored.pairing_id).is_err()
            || stored.encrypted_user_data.len() > 8192
        {
            return Err(PairingError::InvalidResponse);
        }
        Ok(Self {
            keys: super::keys::GaiaKeys::restore(&stored.first, &stored.second)?,
            _peer: std::mem::take(&mut stored.peer),
            _pairing_id: std::mem::take(&mut stored.pairing_id),
            _encrypted_user_data: Zeroizing::new(std::mem::take(&mut stored.encrypted_user_data)),
        })
    }

    /// Offline encryption only; it makes no messaging request and advertises
    /// no send capability. Keys never leave the confirmed pairing object.
    pub fn encrypt_payload(
        &self,
        plaintext: &[u8],
    ) -> Result<super::cipher::EncryptedPayload, super::cipher::CipherError> {
        self.keys.encrypt(plaintext)
    }

    pub fn decrypt_payload(
        &self,
        ciphertext: &[u8],
    ) -> Result<super::cipher::Plaintext, super::cipher::CipherError> {
        self.keys.decrypt(ciphertext)
    }
}

#[derive(Message)]
struct FinalResponse {
    #[prost(int32, tag = "1")]
    status: i32,
    #[prost(string, tag = "4")]
    pairing_id: String,
    #[prost(bytes = "vec", tag = "8")]
    encrypted_user_data: Vec<u8>,
}

impl Drop for FinalResponse {
    fn drop(&mut self) {
        self.encrypted_user_data.zeroize();
    }
}

// Independently authored projections of Lw/Cza, sza/Aza, tza/Bza, and Dza/Eza.
// Only fields needed by the observed H5a/G5a flow are represented.
#[derive(Message)]
struct PairingRequest {
    #[prost(string, tag = "1")]
    pairing_id: String,
    #[prost(message, optional, tag = "2")]
    device: Option<DeviceInfo>,
    #[prost(int64, tag = "3")]
    timestamp_millis: i64,
    #[prost(bytes = "vec", optional, tag = "4")]
    handshake: Option<Vec<u8>>,
    #[prost(int32, tag = "5")]
    auth_revision: i32,
    #[prost(int32, tag = "6")]
    key_revision: i32,
}

#[derive(Message)]
struct DeviceInfo {
    #[prost(string, tag = "1")]
    user_agent: String,
    #[prost(int32, tag = "2")]
    browser: i32,
    #[prost(string, tag = "3")]
    platform: String,
    #[prost(bool, tag = "5")]
    mobile: bool,
    #[prost(int32, tag = "6")]
    form_factor: i32,
}

#[cfg(test)]
#[derive(Message)]
struct HandshakeMessage {
    #[prost(int32, tag = "1")]
    message_type: i32,
    #[prost(bytes = "vec", tag = "2")]
    message_data: Vec<u8>,
}

#[derive(Message)]
struct InitialResponse {
    #[prost(int32, tag = "1")]
    status: i32,
    #[prost(bool, tag = "3")]
    confirmation_required: bool,
    #[prost(string, tag = "4")]
    pairing_id: String,
    // Preserve embedded protobuf bytes exactly for the UKEY2 transcript.
    #[prost(bytes = "vec", optional, tag = "5")]
    handshake: Option<Vec<u8>>,
    #[prost(int32, tag = "6")]
    auth_revision: i32,
    #[prost(int32, tag = "7")]
    key_revision: i32,
    #[prost(int32, tag = "9")]
    feature_revision: i32,
}

fn envelope(pairing_id: &str, timestamp_millis: i64, handshake: &[u8], initial: bool) -> Vec<u8> {
    PairingRequest {
        pairing_id: pairing_id.to_owned(),
        device: Some(DeviceInfo {
            user_agent: concat!("Handover/", env!("CARGO_PKG_VERSION")).to_owned(),
            browser: 1,
            platform: "Linux".to_owned(),
            mobile: false,
            form_factor: 1,
        }),
        timestamp_millis,
        handshake: Some(handshake.to_vec()),
        auth_revision: i32::from(initial),
        key_revision: i32::from(initial),
    }
    .encode_to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{PhoneSelection, RegisteredSources};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use crypto_provider_rustcrypto::RustCryptoImpl;
    use rand::{SeedableRng, rngs::StdRng};
    use serde_json::json;
    use std::collections::HashSet;
    use ukey2_rs::{HandshakeImplementation, StateMachine, Ukey2ServerStage1, Ukey2ServerStage2};

    fn attempt() -> InitialPairing {
        let record = json!([
            STANDARD.encode("phone"),
            null,
            1,
            null,
            null,
            null,
            null,
            STANDARD.encode([8, 1, 16, 1])
        ]);
        let body = serde_json::to_vec(&json!([[], null, [null, null, [record]]])).unwrap();
        let sources = RegisteredSources::from_lookup_response(&body).unwrap();
        let PhoneSelection::Selected(phone) = sources.select_phone() else {
            panic!()
        };
        InitialPairing::prepare(phone).unwrap()
    }

    fn registration() -> crate::registration::UnpairedRegistration {
        crate::registration::RegistrationAttempt::prepare()
            .unwrap()
            .accept_response(
                br#"[[],"cGhvbmUtaWQ=",null,["c3ludGhldGljLXRva2Vu","3600000000"]]"#,
                Duration::from_secs(3600),
            )
            .unwrap()
    }

    fn exchange(
        attempt: &InitialPairing,
    ) -> (InitialResponse, Ukey2ServerStage2<RustCryptoImpl<StdRng>>) {
        let mut rng = StdRng::from_entropy();
        let peer = Ukey2ServerStage1::<RustCryptoImpl<StdRng>>::from(
            HashSet::from(["AES_256_CBC-HMAC_SHA256".to_owned()]),
            HandshakeImplementation::PublicKeyInProtobuf,
        )
        .advance_state(&mut rng, attempt.handshake.client_init().unwrap())
        .unwrap();
        let response = InitialResponse {
            status: 0,
            confirmation_required: true,
            pairing_id: attempt.pairing_id.clone(),
            handshake: Some(peer.server_init_msg().to_vec()),
            auth_revision: 1,
            key_revision: 1,
            feature_revision: 0,
        };
        (response, peer)
    }

    fn response(attempt: &InitialPairing) -> InitialResponse {
        exchange(attempt).0
    }

    #[test]
    fn initial_exchange_prepares_distinct_finish_without_claiming_confirmation() {
        let attempt = attempt();
        let initial = PairingRequest::decode(attempt.request_bytes().unwrap()).unwrap();
        assert_eq!(initial.auth_revision, 1);
        assert_eq!(initial.key_revision, 1);
        assert_eq!(
            HandshakeMessage::decode(initial.handshake.as_ref().unwrap().as_slice())
                .unwrap()
                .message_type,
            2
        );
        assert_eq!(initial.device.as_ref().unwrap().platform, "Linux");
        assert!(initial.timestamp_millis > 0);
        let old_request_id = attempt.request_id().to_owned();
        let (response, peer) = exchange(&attempt);
        let response = response.encode_to_vec();
        let pending = attempt
            .accept_response(&old_request_id, b"phone", &response)
            .unwrap();
        let finish = PairingRequest::decode(pending.request_bytes().unwrap()).unwrap();
        assert_eq!(finish.pairing_id, initial.pairing_id);
        assert_eq!(finish.timestamp_millis, initial.timestamp_millis);
        assert_eq!(finish.auth_revision, 0);
        assert_eq!(finish.key_revision, 0);
        let finish_handshake = finish.handshake.unwrap();
        assert_eq!(
            HandshakeMessage::decode(finish_handshake.as_slice())
                .unwrap()
                .message_type,
            4
        );
        assert_ne!(pending.request_id(), old_request_id);
        assert_eq!(pending.auth_string().unwrap().len(), 32);
        let peer = peer
            .advance_state(&mut StdRng::from_entropy(), &finish_handshake)
            .unwrap();
        assert!(
            pending.auth_string().unwrap()
                == &peer
                    .completed_handshake()
                    .auth_string::<RustCryptoImpl<StdRng>>()
                    .derive_array::<32>()
                    .unwrap()
        );
        assert_eq!(
            format!("{pending:?}"),
            "AwaitingPhoneConfirmation { redacted }"
        );
    }

    #[test]
    fn outer_pairing_envelopes_route_only_to_the_selected_phone() {
        let session_id = String::new();
        let attempt = attempt();
        let registration = registration();
        let request_id = attempt.request_id().to_owned();
        let initial_bytes = attempt.request_bytes().unwrap().to_vec();
        let envelope = registration
            .initial_pairing_envelope(&attempt, "person@example.test", &session_id)
            .unwrap();
        let sent = MessagingSend::decode(envelope.as_bytes()).unwrap();
        let destination = sent.destination.as_ref().unwrap();
        assert_eq!(destination.kind, 16);
        assert_eq!(destination.id, "person@example.test");
        assert_eq!(destination.client, "GDitto");
        assert_eq!(sent.recipient_identities, [b"phone".to_vec()]);
        assert_eq!(sent.maximum_lifetime_micros, 300_000_000);
        let auth = sent.authentication.as_ref().unwrap();
        assert_eq!(auth.request_id, request_id);
        assert_eq!(auth.application, "GDitto");
        assert_eq!(auth.registration_token, b"synthetic-token");
        let client_info = auth.client_info.as_ref().unwrap();
        assert_eq!(client_info.wire_year, 20261001);
        assert_eq!(client_info.wire_major, 2);
        assert_eq!(client_info.wire_minor, 0);
        assert_eq!(client_info.client_type, 4);
        assert_eq!(client_info.platform_type, 6);
        let message = sent.message.as_ref().unwrap();
        assert_eq!(message.request_id, request_id);
        assert_eq!(message.kind, 19);
        assert_eq!(message.routing.as_ref().unwrap().delivery_class, 20);
        let wrapper = PairingRequestWrapper::decode(message.payload.as_slice()).unwrap();
        assert_eq!(wrapper.request_id, request_id);
        assert_eq!(wrapper.request_type, 44);
        assert_eq!(wrapper.request, initial_bytes);
        assert_eq!(wrapper.session_id, session_id);
        assert_eq!(format!("{envelope:?}"), "PairingSendEnvelope { redacted }");

        let (response, _) = exchange(&attempt);
        let pending = attempt
            .accept_response(&request_id, b"phone", &response.encode_to_vec())
            .unwrap();
        let finish_id = pending.request_id().to_owned();
        let finish_bytes = pending.request_bytes().unwrap().to_vec();
        let envelope = registration
            .confirmation_envelope(&pending, "person@example.test", &session_id)
            .unwrap();
        let sent = MessagingSend::decode(envelope.as_bytes()).unwrap();
        let message = sent.message.as_ref().unwrap();
        assert_eq!(message.request_id, finish_id);
        assert_eq!(message.routing.as_ref().unwrap().delivery_class, 2);
        let wrapper = PairingRequestWrapper::decode(message.payload.as_slice()).unwrap();
        assert_eq!(wrapper.request_type, 45);
        assert_eq!(wrapper.request, finish_bytes);
    }

    #[test]
    fn outer_pairing_envelope_rejects_bad_account_and_session_routes() {
        let attempt = attempt();
        let registration = registration();
        let session_id = Uuid::new_v4().to_string();
        for account in ["", "name\nprivate", &"x".repeat(321)] {
            assert_eq!(
                registration
                    .initial_pairing_envelope(&attempt, account, &session_id)
                    .unwrap_err(),
                crate::registration::RegistrationError::Pairing(PairingError::InvalidRouting)
            );
        }
        assert_eq!(
            registration
                .initial_pairing_envelope(&attempt, "person@example.test", "invalid")
                .unwrap_err(),
            crate::registration::RegistrationError::Pairing(PairingError::InvalidRouting)
        );
    }

    #[test]
    fn pairing_send_envelope_expires_before_transport() {
        let attempt = attempt();
        let registration = registration();
        let mut envelope = registration
            .initial_pairing_envelope(&attempt, "person@example.test", &Uuid::new_v4().to_string())
            .unwrap();
        envelope.started = Instant::now() - Duration::from_secs(301);
        assert_eq!(envelope.ensure_valid(), Err(PairingError::Expired));
    }

    #[tokio::test]
    async fn pairing_http_transport_reports_only_http_acceptance() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let attempt = attempt();
        let session = Uuid::new_v4().to_string();
        let registration = crate::registration::RegistrationAttempt::prepare()
            .unwrap()
            .accept_response(
                br#"[[],"cGhvbmUtaWQ=",null,["c3ludGhldGljLXRva2Vu","3600000000"]]"#,
                std::time::Duration::from_secs(60),
            )
            .unwrap();
        let envelope = registration
            .initial_pairing_envelope(&attempt, "person@example.test", &session)
            .unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}{}",
            listener.local_addr().unwrap(),
            crate::SEND_MESSAGE_PATH
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 2048];
            loop {
                let count = stream.read(&mut buffer).await.unwrap();
                assert_ne!(count, 0);
                request.extend_from_slice(&buffer[..count]);
                if let Some(offset) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = std::str::from_utf8(&request[..offset]).unwrap();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= offset + 4 + length {
                        break;
                    }
                }
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json+protobuf\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]").await.unwrap();
            request
        });

        let proof = crate::BrowserProof {
            kind: "gaia_pairing".into(),
            endpoint: crate::ENDPOINTS[0].into(),
            origin: crate::ORIGIN_VALUE.into(),
            authorization: "synthetic-auth".into(),
            api_key: "synthetic-api-key".into(),
            auth_user: Some("0".into()),
            service_cookie: Some("SID=synthetic-cookie".into()),
            account_email: Some("person@example.test".into()),
            browser_request: None,
        };
        let mut headers = proof.validate_messaging().unwrap();
        headers.insert(
            crate::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/x-protobuf"),
        );
        let result = crate::post_pairing_envelope(
            &crate::client(false).unwrap(),
            &endpoint,
            headers,
            &envelope,
        )
        .await
        .unwrap();
        assert_eq!(result.http_status, 200);
        let request = server.await.unwrap();
        let offset = request
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap();
        let headers = std::str::from_utf8(&request[..offset])
            .unwrap()
            .to_ascii_lowercase();
        assert!(headers.starts_with(&format!(
            "post {} http/1.1",
            crate::SEND_MESSAGE_PATH.to_ascii_lowercase()
        )));
        assert!(headers.contains("content-type: application/json+protobuf"));
        assert!(headers.contains("authorization: synthetic-auth"));
        assert!(headers.contains("cookie: sid=synthetic-cookie"));
        let sent: serde_json::Value = serde_json::from_slice(&request[offset + 4..]).unwrap();
        assert_eq!(sent[0], json!([16, "person@example.test", "GDitto"]));
        assert_eq!(sent[1][0], attempt.request_id());
        assert_eq!(sent[1][1], 19);
        assert_eq!(sent[1][22], json!([null, 20]));
        assert_eq!(sent[2][0], attempt.request_id());
        assert_eq!(sent[2][2], "GDitto");
        assert_eq!(
            STANDARD.decode(sent[2][5].as_str().unwrap()).unwrap(),
            b"synthetic-token"
        );
        assert_eq!(sent[4], "300000000");
        assert_eq!(sent[8], json!([STANDARD.encode("phone")]));
        let wrapper = PairingRequestWrapper::decode(
            STANDARD
                .decode(sent[1][11].as_str().unwrap())
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        assert_eq!(wrapper.request_type, 44);
        assert_eq!(
            format!("{result:?}"),
            "SendMessageAccepted { http_status: 200 }"
        );
    }

    #[test]
    fn rejects_wrong_outer_request_sender_or_inner_pairing_id() {
        for mismatch in 0..3 {
            let attempt = attempt();
            let mut response = response(&attempt);
            let mut id = attempt.request_id().to_owned();
            let mut sender = b"phone".to_vec();
            match mismatch {
                0 => id.push('x'),
                1 => sender.push(0),
                _ => response.pairing_id.push('x'),
            }
            assert!(matches!(
                attempt.accept_response(&id, &sender, &response.encode_to_vec()),
                Err(PairingError::Correlation)
            ));
        }
    }

    #[test]
    fn rejects_status_confirmation_revisions_and_missing_handshake() {
        for invalid in 0..8 {
            let attempt = attempt();
            let mut response = response(&attempt);
            let id = attempt.request_id().to_owned();
            let expected = match invalid {
                0 => {
                    response.status = 1;
                    PairingError::Rejected
                }
                1 => {
                    response.confirmation_required = false;
                    PairingError::ConfirmationRequired
                }
                2 => {
                    response.handshake = None;
                    PairingError::InvalidResponse
                }
                3 => {
                    response.auth_revision = 0;
                    PairingError::UnsupportedRevision
                }
                4 => {
                    response.auth_revision = 2;
                    PairingError::UnsupportedRevision
                }
                5 => {
                    response.key_revision = 0;
                    PairingError::UnsupportedRevision
                }
                6 => {
                    response.key_revision = -1;
                    PairingError::UnsupportedRevision
                }
                _ => {
                    response.feature_revision = 1;
                    PairingError::UnsupportedRevision
                }
            };
            assert!(
                matches!(attempt.accept_response(&id, b"phone", &response.encode_to_vec()), Err(e) if e == expected)
            );
        }
    }

    #[test]
    fn malformed_bounded_response_never_advances_handshake() {
        for body in [vec![], vec![0xff], vec![0; RESPONSE_LIMIT + 1]] {
            let attempt = attempt();
            let id = attempt.request_id().to_owned();
            assert!(matches!(
                attempt.accept_response(&id, b"phone", &body),
                Err(PairingError::InvalidResponse)
            ));
        }
    }

    #[test]
    fn embedded_handshake_keeps_unknown_fields_in_exact_transcript() {
        let attempt = attempt();
        let mut response = response(&attempt);
        response
            .handshake
            .as_mut()
            .unwrap()
            .extend_from_slice(&[0x78, 1]);
        let encoded = response.encode_to_vec();
        let decoded = InitialResponse::decode(encoded.as_slice()).unwrap();
        assert!(decoded.handshake == response.handshake);
        let id = attempt.request_id().to_owned();
        assert!(attempt.accept_response(&id, b"phone", &encoded).is_ok());
    }

    #[test]
    fn expiry_blocks_initial_response_and_finish_access() {
        use std::time::Instant;
        let mut attempt = attempt();
        let response = response(&attempt).encode_to_vec();
        let id = attempt.request_id().to_owned();
        attempt.handshake.started = Instant::now() - super::super::HANDSHAKE_LIFETIME;
        assert_eq!(
            attempt.request_bytes().unwrap_err(),
            PairingError::Handshake(HandshakeError::Expired)
        );
        assert!(matches!(
            attempt.accept_response(&id, b"phone", &response),
            Err(PairingError::Handshake(HandshakeError::Expired))
        ));

        let attempt = self::attempt();
        let response = self::response(&attempt).encode_to_vec();
        let id = attempt.request_id().to_owned();
        let mut pending = attempt.accept_response(&id, b"phone", &response).unwrap();
        pending.pending.started = Instant::now() - super::super::HANDSHAKE_LIFETIME;
        assert_eq!(
            pending.request_bytes().unwrap_err(),
            PairingError::Handshake(HandshakeError::Expired)
        );
        assert_eq!(
            pending.auth_string().unwrap_err(),
            PairingError::Handshake(HandshakeError::Expired)
        );
        assert_eq!(
            pending.verification_emoji().unwrap_err(),
            PairingError::Handshake(HandshakeError::Expired)
        );
    }

    #[test]
    fn final_response_requires_correlated_success_and_retains_opaque_data() {
        for invalid in 0..7 {
            let attempt = attempt();
            let response = response(&attempt).encode_to_vec();
            let id = attempt.request_id().to_owned();
            let mut pending = attempt.accept_response(&id, b"phone", &response).unwrap();
            let mut final_response = FinalResponse {
                status: 0,
                pairing_id: pending.pairing_id.clone(),
                encrypted_user_data: vec![1, 2, 3],
            };
            let mut id = pending.request_id().to_owned();
            let mut sender = b"phone".to_vec();
            match invalid {
                1 => id.push('x'),
                2 => sender.push(0),
                3 => final_response.pairing_id.push('x'),
                4 => final_response.status = 1,
                5 => final_response.encrypted_user_data = vec![0; 8193],
                6 => {
                    pending.pending.started =
                        std::time::Instant::now() - super::super::HANDSHAKE_LIFETIME
                }
                _ => {}
            }
            let result = pending.accept_response(&id, &sender, &final_response.encode_to_vec());
            if invalid == 0 {
                let confirmed = result.unwrap();
                assert!(confirmed._encrypted_user_data.as_slice() == [1, 2, 3]);
                assert_eq!(
                    format!("{confirmed:?}"),
                    "PhoneConfirmedPairing { redacted }"
                );
                let ciphertext = confirmed
                    .encrypt_payload(b"synthetic protocol bytes")
                    .unwrap();
                assert!(
                    confirmed
                        .decrypt_payload(ciphertext.as_bytes())
                        .unwrap()
                        .as_bytes()
                        == b"synthetic protocol bytes"
                );
            } else {
                assert!(result.is_err());
            }
        }
    }
}

// Private local record, not a Google wire type or helper IPC payload.
#[derive(Message)]
struct StoredPairing {
    #[prost(uint32, tag = "1")]
    version: u32,
    #[prost(bytes = "vec", tag = "2")]
    first: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    second: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    peer: Vec<u8>,
    #[prost(string, tag = "5")]
    pairing_id: String,
    #[prost(bytes = "vec", tag = "6")]
    encrypted_user_data: Vec<u8>,
}

impl Drop for StoredPairing {
    fn drop(&mut self) {
        self.first.zeroize();
        self.second.zeroize();
        self.peer.zeroize();
        self.pairing_id.zeroize();
        self.encrypted_user_data.zeroize();
    }
}
