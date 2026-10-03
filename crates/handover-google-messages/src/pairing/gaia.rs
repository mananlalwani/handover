//! Offline Gaia pairing envelopes and correlated phone confirmation.
//! Transport and emoji display remain absent; no type creates an online account.

use prost::Message;
use std::{
    fmt,
    time::{SystemTime, UNIX_EPOCH},
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

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Inner request for message type 44. An eventual transport must supply the
    /// separate routing envelope and check registration expiry before sending.
    pub fn request_bytes(&self) -> Result<&[u8], PairingError> {
        self.handshake.client_init()?;
        Ok(&self.request)
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

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Inner client-finish request for message type 45, using the original
    /// pairing ID and timestamp but a fresh outer request ID.
    pub fn request_bytes(&self) -> Result<&[u8], PairingError> {
        self.pending.client_finish()?;
        Ok(&self.request)
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
