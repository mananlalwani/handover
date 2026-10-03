//! Offline account registration preparation. This module makes no HTTP requests.

use base64::{Engine, engine::general_purpose};
use prost::Message;
use rand::{RngCore, rngs::OsRng};
use serde_json::{Value, json};
use std::{
    fmt,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::pairing::gaia::{
    AwaitingPhoneConfirmation, InitialPairing, PairingError, PairingSendEnvelope,
};
use crate::{RESPONSE_LIMIT, request_header};
const ID_LIMIT: usize = 1024;
const TOKEN_LIMIT: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationError {
    InvalidResponse,
    ResponseTooLarge,
    InvalidLifetime,
    Expired,
    Encoding,
    Pairing(PairingError),
    Acknowledgement(crate::receive::ReceiveError),
    SessionStore(crate::session_store::SessionStoreError),
    InvalidStoredRegistration,
}

/// One fresh device identity and transport key, for one registration attempt.
/// Debug and Serde cannot expose the request or key.
pub struct RegistrationAttempt {
    device_id: String,
    transport_key: Zeroizing<[u8; 32]>,
    request: Zeroizing<Vec<u8>>,
    started: Instant,
}

impl fmt::Debug for RegistrationAttempt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistrationAttempt { redacted }")
    }
}

impl RegistrationAttempt {
    pub fn prepare() -> Result<Self, RegistrationError> {
        let device_id = format!("messages-web-{}", Uuid::new_v4().simple());
        let mut transport_key = Zeroizing::new([0; 32]);
        OsRng.fill_bytes(transport_key.as_mut());
        let mut metadata = DeviceMetadata {
            transport_key: transport_key.to_vec(),
        };
        let encoded = Zeroizing::new(metadata.encode_to_vec());
        metadata.transport_key.zeroize();
        let mut device = vec![Value::Null; 36];
        device[0] = json!([3, device_id]);
        device[35] = json!(general_purpose::STANDARD.encode(encoded.as_slice()));
        // gs sets mode 0 with the default omitted. Field 4 keeps the array at
        // four positions; the mode slot is null, not a populated mode-1 lookup.
        let mut body = json!([request_header(), device, null, "GDitto"]);
        let bytes = serde_json::to_vec(&body).map_err(|_| RegistrationError::Encoding);
        if let Some(Value::String(secret)) = body.get_mut(1).and_then(|v| v.get_mut(35)) {
            secret.zeroize();
        }
        Ok(Self {
            device_id,
            transport_key,
            request: Zeroizing::new(bytes?),
            started: Instant::now(),
        })
    }

    /// A future registration transport may send these bytes once. No retries
    /// or network effects are provided by this API.
    pub fn request_bytes(&self) -> &[u8] {
        self.request.as_slice()
    }

    /// Consume the attempt. A returned token represents registration only,
    /// never successful pairing. The caller must provide its local lifetime cap.
    pub fn accept_response(
        self,
        body: &[u8],
        maximum_lifetime: Duration,
    ) -> Result<UnpairedRegistration, RegistrationError> {
        if maximum_lifetime.is_zero() {
            return Err(RegistrationError::InvalidLifetime);
        }
        if body.len() > RESPONSE_LIMIT {
            return Err(RegistrationError::ResponseTooLarge);
        }
        let mut value: Value =
            serde_json::from_slice(body).map_err(|_| RegistrationError::InvalidResponse)?;
        let result = (|| {
            let fields = value.as_array().ok_or(RegistrationError::InvalidResponse)?;
            if fields.len() != 4 || !fields[0].is_array() {
                return Err(RegistrationError::InvalidResponse);
            }
            let identity = decode_bytes(&fields[1], ID_LIMIT)?;
            if identity.is_empty() {
                return Err(RegistrationError::InvalidResponse);
            }
            let token_fields = fields[3]
                .as_array()
                .ok_or(RegistrationError::InvalidResponse)?;
            if token_fields.len() != 2 {
                return Err(RegistrationError::InvalidResponse);
            }
            let token = decode_bytes(&token_fields[0], TOKEN_LIMIT)?;
            if token.is_empty() {
                return Err(RegistrationError::InvalidResponse);
            }
            let micros = match &token_fields[1] {
                Value::String(s)
                    if !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) =>
                {
                    s.parse::<u64>().ok()
                }
                n => n.as_u64(),
            }
            .filter(|n| *n > 0 && *n <= i64::MAX as u64)
            .ok_or(RegistrationError::InvalidLifetime)?;
            let lifetime = Duration::from_micros(micros).min(maximum_lifetime);
            if self.started.elapsed() >= lifetime {
                return Err(RegistrationError::Expired);
            }
            Ok(UnpairedRegistration {
                _device_id: self.device_id,
                _identity: identity,
                _token: token,
                _transport_key: self.transport_key,
                started: self.started,
                lifetime,
            })
        })();
        erase_strings(&mut value);
        result
    }
}

/// Credential material is unpaired and private to this protocol crate.
/// There is no online state, persistence, or authenticated-session conversion.
pub struct UnpairedRegistration {
    _device_id: String,
    _identity: Zeroizing<Vec<u8>>,
    _token: Zeroizing<Vec<u8>>,
    _transport_key: Zeroizing<[u8; 32]>,
    started: Instant,
    lifetime: Duration,
}

impl fmt::Debug for UnpairedRegistration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UnpairedRegistration { redacted }")
    }
}

impl UnpairedRegistration {
    /// Persist this incomplete registration in the restricted local session
    /// store so an interrupted pairing does not discard its server token.
    pub fn persist_pending(
        &self,
        store: &crate::session_store::SessionStore,
    ) -> Result<(), RegistrationError> {
        let remaining = self.remaining_lifetime()?;
        let expires_unix = SystemTime::now()
            .checked_add(remaining)
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .ok_or(RegistrationError::InvalidLifetime)?;
        let mut encoded = StoredUnpairedRegistration {
            version: 1,
            device_id: self._device_id.clone(),
            identity: self._identity.to_vec(),
            token: self._token.to_vec(),
            transport_key: self._transport_key.to_vec(),
            expires_unix,
        }
        .encode_to_vec();
        let result = (|| {
            let record = crate::session_store::SessionRecord::new(encoded.clone())
                .map_err(RegistrationError::SessionStore)?;
            let key = crate::session_store::account_key_for_identity(&self._identity)
                .map_err(RegistrationError::SessionStore)?;
            store
                .store(&key, &record)
                .map_err(RegistrationError::SessionStore)
        })();
        zeroize::Zeroize::zeroize(&mut encoded);
        result
    }

    /// Restore a pending registration for the same opaque server identity.
    /// Expired or malformed records are rejected and never become accounts.
    pub fn restore_pending(
        store: &crate::session_store::SessionStore,
        identity: &[u8],
    ) -> Result<Option<Self>, RegistrationError> {
        let Some(record) = store
            .load_for_identity(identity)
            .map_err(RegistrationError::SessionStore)?
        else {
            return Ok(None);
        };
        let mut saved = StoredUnpairedRegistration::decode(record.as_bytes())
            .map_err(|_| RegistrationError::InvalidStoredRegistration)?;
        let restored = (|| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| RegistrationError::InvalidStoredRegistration)?
                .as_secs();
            if saved.version != 1
                || saved.identity != identity
                || saved.device_id.len() > 128
                || !saved.device_id.starts_with("messages-web-")
                || saved.token.is_empty()
                || saved.token.len() > TOKEN_LIMIT
                || saved.transport_key.len() != 32
                || saved.expires_unix <= now
                || saved.expires_unix.saturating_sub(now) > 30 * 24 * 60 * 60
            {
                return Err(RegistrationError::InvalidStoredRegistration);
            }
            let lifetime = Duration::from_secs(saved.expires_unix - now);
            let mut transport_key = Zeroizing::new([0; 32]);
            transport_key.copy_from_slice(&saved.transport_key);
            Ok(Self {
                _device_id: std::mem::take(&mut saved.device_id),
                _identity: Zeroizing::new(std::mem::take(&mut saved.identity)),
                _token: Zeroizing::new(std::mem::take(&mut saved.token)),
                _transport_key: transport_key,
                started: Instant::now(),
                lifetime,
            })
        })();
        saved.zeroize_secrets();
        restored.map(Some)
    }

    /// Prepare a fresh receive stream request using this unpaired credential.
    /// This does not start a stream, refresh a token, or establish online state.
    pub fn prepare_receive(&self) -> Result<ReceiveRequest, RegistrationError> {
        self.remaining_lifetime()?;
        let mut header = request_header();
        header[5] = Value::String(general_purpose::STANDARD.encode(self._token.as_slice()));
        // wLa field 4 is an empty uLa cursor for a fresh receive stream.
        let mut body = json!([header, null, null, []]);
        let encoded = serde_json::to_vec(&body).map_err(|_| RegistrationError::Encoding);
        erase_strings(&mut body);
        Ok(ReceiveRequest {
            bytes: Zeroizing::new(encoded?),
            started: self.started,
            lifetime: self.lifetime,
        })
    }

    /// Build a fresh initial pairing envelope only while this registration
    /// token remains valid. The token never leaves this registration object.
    pub fn initial_pairing_envelope(
        &self,
        pairing: &InitialPairing,
        account_id: &str,
        session_id: &str,
    ) -> Result<PairingSendEnvelope, RegistrationError> {
        let lifetime = self.remaining_lifetime()?;
        pairing
            .send_envelope_with_lifetime(account_id, session_id, self._token.as_slice(), lifetime)
            .map_err(RegistrationError::Pairing)
    }

    /// Build a fresh final pairing envelope only while this registration
    /// token remains valid.
    pub fn confirmation_envelope(
        &self,
        pairing: &AwaitingPhoneConfirmation,
        account_id: &str,
        session_id: &str,
    ) -> Result<PairingSendEnvelope, RegistrationError> {
        let lifetime = self.remaining_lifetime()?;
        pairing
            .send_envelope_with_lifetime(account_id, session_id, self._token.as_slice(), lifetime)
            .map_err(RegistrationError::Pairing)
    }

    pub fn acknowledgement_request(
        &self,
        batch: &crate::receive::AckBatch,
    ) -> Result<crate::receive::AckRequest, RegistrationError> {
        let lifetime = self.remaining_lifetime()?;
        batch
            .request(self._token.as_slice(), lifetime)
            .map_err(RegistrationError::Acknowledgement)
    }

    pub fn remaining_lifetime(&self) -> Result<Duration, RegistrationError> {
        let remaining = self.lifetime.saturating_sub(self.started.elapsed());
        if remaining.is_zero() {
            Err(RegistrationError::Expired)
        } else {
            Ok(remaining)
        }
    }
}

#[derive(Message)]
struct StoredUnpairedRegistration {
    #[prost(uint32, tag = "1")]
    version: u32,
    #[prost(string, tag = "2")]
    device_id: String,
    #[prost(bytes = "vec", tag = "3")]
    identity: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    token: Vec<u8>,
    #[prost(bytes = "vec", tag = "5")]
    transport_key: Vec<u8>,
    #[prost(uint64, tag = "6")]
    expires_unix: u64,
}

impl StoredUnpairedRegistration {
    fn zeroize_secrets(&mut self) {
        self.identity.zeroize();
        self.token.zeroize();
        self.transport_key.zeroize();
        self.device_id.zeroize();
    }
}

impl Drop for StoredUnpairedRegistration {
    fn drop(&mut self) {
        self.zeroize_secrets();
    }
}

/// Encoded authenticated request, kept out of diagnostics and serialization.
pub struct ReceiveRequest {
    bytes: Zeroizing<Vec<u8>>,
    started: Instant,
    lifetime: Duration,
}

impl fmt::Debug for ReceiveRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReceiveRequest { redacted }")
    }
}

impl ReceiveRequest {
    /// Check expiry immediately before handing bytes to the transport.
    pub fn request_bytes(&self) -> Result<&[u8], RegistrationError> {
        if self.started.elapsed() >= self.lifetime {
            return Err(RegistrationError::Expired);
        }
        Ok(self.bytes.as_slice())
    }
}

// Gza field 3, observed in rJ.Xr. Independently authored partial message.
#[derive(Message)]
struct DeviceMetadata {
    #[prost(bytes = "vec", tag = "3")]
    transport_key: Vec<u8>,
}

fn decode_bytes(value: &Value, limit: usize) -> Result<Zeroizing<Vec<u8>>, RegistrationError> {
    let encoded = value
        .as_str()
        .filter(|s| s.len() <= limit.div_ceil(3) * 4)
        .ok_or(RegistrationError::InvalidResponse)?;
    for engine in [
        general_purpose::STANDARD,
        general_purpose::STANDARD_NO_PAD,
        general_purpose::URL_SAFE,
        general_purpose::URL_SAFE_NO_PAD,
    ] {
        if let Ok(decoded) = engine.decode(encoded) {
            let decoded = Zeroizing::new(decoded);
            if decoded.len() <= limit {
                return Ok(decoded);
            }
        }
    }
    Err(RegistrationError::InvalidResponse)
}

fn erase_strings(value: &mut Value) {
    match value {
        Value::String(s) => s.zeroize(),
        Value::Array(values) => values.iter_mut().for_each(erase_strings),
        Value::Object(values) => values.values_mut().for_each(erase_strings),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> Vec<u8> {
        serde_json::to_vec(&json!([
            [],
            general_purpose::STANDARD.encode("synthetic-id"),
            null,
            [
                general_purpose::STANDARD.encode("synthetic-token"),
                "3600000000"
            ]
        ]))
        .unwrap()
    }

    #[test]
    fn receive_uses_registered_token_and_fresh_request_ids() {
        let registration = RegistrationAttempt::prepare()
            .unwrap()
            .accept_response(&response(), Duration::from_secs(60))
            .unwrap();
        let first = registration.prepare_receive().unwrap();
        let second = registration.prepare_receive().unwrap();
        let first_body: Value = serde_json::from_slice(first.request_bytes().unwrap()).unwrap();
        let second_body: Value = serde_json::from_slice(second.request_bytes().unwrap()).unwrap();
        assert_eq!(first_body.as_array().unwrap().len(), 4);
        assert_eq!(
            first_body[0][5],
            general_purpose::STANDARD.encode("synthetic-token")
        );
        assert_eq!(first_body[0][2], "GDitto");
        assert_ne!(first_body[0][0], second_body[0][0]);
        assert_eq!(first_body[3], json!([]));
        assert!(first_body[1].is_null() && first_body[2].is_null());
        assert_eq!(format!("{first:?}"), "ReceiveRequest { redacted }");
    }

    #[test]
    fn expired_registration_cannot_prepare_receive() {
        let mut registration = RegistrationAttempt::prepare()
            .unwrap()
            .accept_response(&response(), Duration::from_secs(60))
            .unwrap();
        let mut prepared = registration.prepare_receive().unwrap();
        prepared.started = Instant::now() - Duration::from_secs(61);
        assert!(matches!(
            prepared.request_bytes(),
            Err(RegistrationError::Expired)
        ));
        registration.started = Instant::now() - Duration::from_secs(61);
        assert!(matches!(
            registration.prepare_receive(),
            Err(RegistrationError::Expired)
        ));
    }

    #[test]
    fn pending_registration_round_trips_only_through_restricted_store() {
        let directory = tempfile::tempdir().unwrap();
        let store = crate::session_store::SessionStore::new(directory.path().join("sessions"));
        let registration = RegistrationAttempt::prepare()
            .unwrap()
            .accept_response(&response(), Duration::from_secs(3600))
            .unwrap();
        let identity = registration._identity.to_vec();
        registration.persist_pending(&store).unwrap();
        let restored = UnpairedRegistration::restore_pending(&store, &identity)
            .unwrap()
            .unwrap();
        assert_eq!(format!("{restored:?}"), "UnpairedRegistration { redacted }");
        let request: Value =
            serde_json::from_slice(restored.prepare_receive().unwrap().request_bytes().unwrap())
                .unwrap();
        assert_eq!(
            request[0][5],
            general_purpose::STANDARD.encode("synthetic-token")
        );
        assert!(
            UnpairedRegistration::restore_pending(&store, b"other-identity")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn pairing_envelope_uses_private_registration_token_and_checks_expiry() {
        let source = json!([
            general_purpose::STANDARD.encode("synthetic-phone"),
            null,
            1,
            null,
            null,
            null,
            null,
            general_purpose::STANDARD.encode([8, 1, 16, 1])
        ]);
        let lookup = serde_json::to_vec(&json!([[], null, [null, null, [source]]])).unwrap();
        let sources = crate::sources::RegisteredSources::from_lookup_response(&lookup).unwrap();
        let crate::sources::PhoneSelection::Selected(phone) = sources.select_phone() else {
            panic!("synthetic source is eligible")
        };
        let pairing = crate::pairing::gaia::InitialPairing::prepare(phone).unwrap();
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut registration = RegistrationAttempt::prepare()
            .unwrap()
            .accept_response(&response(), Duration::from_secs(60))
            .unwrap();
        let envelope = registration
            .initial_pairing_envelope(&pairing, "person@example.test", &session_id)
            .unwrap();
        assert!(!envelope.as_bytes().is_empty());
        assert_eq!(
            format!("{registration:?}"),
            "UnpairedRegistration { redacted }"
        );
        registration.started = Instant::now() - Duration::from_secs(61);
        assert_eq!(
            registration
                .initial_pairing_envelope(&pairing, "person@example.test", &session_id)
                .unwrap_err(),
            RegistrationError::Expired
        );
    }

    #[test]
    fn prepares_owned_registration_with_fresh_key_and_no_lookup_mode() {
        let a = RegistrationAttempt::prepare().unwrap();
        let b = RegistrationAttempt::prepare().unwrap();
        let value: Value = serde_json::from_slice(a.request_bytes()).unwrap();
        assert!(value[2].is_null());
        assert_eq!(value[3], "GDitto");
        assert_eq!(value[0][2], "GDitto");
        assert!(value[0][5].is_null());
        assert_eq!(value[1].as_array().unwrap().len(), 36);
        assert_eq!(value[1][0][0], 3);
        assert_eq!(value[1][0][1], a.device_id);
        let bytes = general_purpose::STANDARD
            .decode(value[1][35].as_str().unwrap())
            .unwrap();
        let metadata = DeviceMetadata::decode(bytes.as_slice()).unwrap();
        assert!(metadata.transport_key.as_slice() == a.transport_key.as_slice());
        assert!(a.device_id != b.device_id);
        assert!(*a.transport_key != *b.transport_key);
        assert_eq!(format!("{a:?}"), "RegistrationAttempt { redacted }");
    }

    #[test]
    fn registration_keeps_secrets_private_and_caps_provider_lifetime() {
        let pending = RegistrationAttempt::prepare()
            .unwrap()
            .accept_response(&response(), Duration::from_secs(120))
            .unwrap();
        assert!(pending.remaining_lifetime().unwrap() <= Duration::from_secs(120));
        assert!(pending._token.as_slice() == b"synthetic-token");
        assert!(pending._identity.as_slice() == b"synthetic-id");
        assert_eq!(format!("{pending:?}"), "UnpairedRegistration { redacted }");
    }

    #[test]
    fn rejects_lookup_response_and_malformed_or_oversized_registration() {
        for body in [
            b"[[],null,[null,null,[]]]".to_vec(),
            b"{}".to_vec(),
            b"[[],\"\",null,[\"\",1]]".to_vec(),
            vec![b' '; RESPONSE_LIMIT + 1],
        ] {
            assert!(
                RegistrationAttempt::prepare()
                    .unwrap()
                    .accept_response(&body, Duration::from_secs(120))
                    .is_err()
            );
        }
        for lifetime in [
            json!(0),
            json!(-1),
            json!(1.5),
            json!("+100"),
            json!("9223372036854775808"),
            json!("18446744073709551616"),
        ] {
            let mut body: Value = serde_json::from_slice(&response()).unwrap();
            body[3][1] = lifetime;
            assert!(
                RegistrationAttempt::prepare()
                    .unwrap()
                    .accept_response(
                        &serde_json::to_vec(&body).unwrap(),
                        Duration::from_secs(120)
                    )
                    .is_err()
            );
        }
    }

    #[test]
    fn delayed_or_expired_response_cannot_create_live_credentials() {
        let mut attempt = RegistrationAttempt::prepare().unwrap();
        attempt.started = Instant::now() - Duration::from_secs(121);
        assert_eq!(
            attempt
                .accept_response(&response(), Duration::from_secs(120))
                .unwrap_err(),
            RegistrationError::Expired
        );
        assert_eq!(
            RegistrationAttempt::prepare()
                .unwrap()
                .accept_response(&response(), Duration::ZERO)
                .unwrap_err(),
            RegistrationError::InvalidLifetime
        );
    }
}
