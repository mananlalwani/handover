//! UKEY2 preparation only. No registration, network traffic, or paired session.

use crypto_provider_rustcrypto::RustCryptoImpl;
use rand::{SeedableRng, rngs::StdRng};
use std::{
    fmt,
    time::{Duration, Instant},
};
use ukey2_rs::{HandshakeImplementation, NextProtocol, StateMachine, Ukey2ClientStage1};
use zeroize::Zeroizing;

type Provider = RustCryptoImpl<StdRng>;
const MESSAGE_LIMIT: usize = 4096;
const HANDSHAKE_LIFETIME: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeError {
    InvalidMessage,
    Expired,
    DerivationFailed,
}

/// A single-use client handshake. Debug excludes transcript and private state.
pub struct PairingHandshake {
    state: Ukey2ClientStage1<Provider>,
    started: Instant,
}

impl fmt::Debug for PairingHandshake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PairingHandshake { redacted }")
    }
}

impl PairingHandshake {
    pub fn start() -> Self {
        let mut rng = StdRng::from_entropy();
        Self {
            state: Ukey2ClientStage1::<Provider>::from_p256(
                &mut rng,
                vec![NextProtocol::Aes256CbcHmacSha256],
                HandshakeImplementation::PublicKeyInProtobuf,
            ),
            started: Instant::now(),
        }
    }

    /// Bytes for a future pairing envelope. This method sends nothing.
    pub fn client_init(&self) -> Result<&[u8], HandshakeError> {
        self.check_expiry()?;
        Ok(self.state.client_init_msg())
    }

    /// Consume the handshake even when the peer message is rejected.
    pub fn accept_server_init(
        self,
        message: &[u8],
    ) -> Result<PendingPhoneConfirmation, HandshakeError> {
        self.check_expiry()?;
        if message.is_empty() || message.len() > MESSAGE_LIMIT {
            return Err(HandshakeError::InvalidMessage);
        }
        let mut rng = StdRng::from_entropy();
        let client = self
            .state
            .advance_state(&mut rng, message)
            .map_err(|_| HandshakeError::InvalidMessage)?;
        let handshake = client.completed_handshake();
        let auth_string = Zeroizing::new(
            handshake
                .auth_string::<Provider>()
                .derive_array::<32>()
                .ok_or(HandshakeError::DerivationFailed)?,
        );
        let next_protocol_secret = Zeroizing::new(
            handshake
                .next_protocol_secret::<Provider>()
                .derive_array::<32>()
                .ok_or(HandshakeError::DerivationFailed)?,
        );
        Ok(PendingPhoneConfirmation {
            client_finish: client.client_finished_msg().to_vec(),
            auth_string,
            _next_protocol_secret: next_protocol_secret,
            started: self.started,
        })
    }

    fn check_expiry(&self) -> Result<(), HandshakeError> {
        check_expiry(self.started)
    }
}

/// Derived material is pending, not authenticated. No API releases session keys.
pub struct PendingPhoneConfirmation {
    client_finish: Vec<u8>,
    auth_string: Zeroizing<[u8; 32]>,
    _next_protocol_secret: Zeroizing<[u8; 32]>,
    started: Instant,
}

impl fmt::Debug for PendingPhoneConfirmation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingPhoneConfirmation { redacted }")
    }
}

impl PendingPhoneConfirmation {
    pub fn client_finish(&self) -> Result<&[u8], HandshakeError> {
        check_expiry(self.started)?;
        Ok(&self.client_finish)
    }

    /// Raw UKEY2 verification material. Emoji mapping is not implemented yet.
    pub fn auth_string(&self) -> Result<&[u8; 32], HandshakeError> {
        check_expiry(self.started)?;
        Ok(&self.auth_string)
    }
}

fn check_expiry(started: Instant) -> Result<(), HandshakeError> {
    if started.elapsed() >= HANDSHAKE_LIFETIME {
        Err(HandshakeError::Expired)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use ukey2_proto::{protobuf::Message, ukey2_all_proto::ukey};
    use ukey2_rs::Ukey2ServerStage1;

    fn server() -> Ukey2ServerStage1<Provider> {
        Ukey2ServerStage1::from(
            HashSet::from(["AES_256_CBC-HMAC_SHA256".to_owned()]),
            HandshakeImplementation::PublicKeyInProtobuf,
        )
    }

    #[test]
    fn observed_p256_offer_completes_with_upstream_peer_and_matching_secrets() {
        let client = PairingHandshake::start();
        let init = client.client_init().unwrap();
        let envelope = ukey::Ukey2Message::parse_from_bytes(init).unwrap();
        let init_fields = ukey::Ukey2ClientInit::parse_from_bytes(envelope.message_data()).unwrap();
        assert_eq!(init_fields.cipher_commitments.len(), 1);
        assert_eq!(
            init_fields.cipher_commitments[0].handshake_cipher(),
            ukey::Ukey2HandshakeCipher::P256_SHA512
        );
        assert_eq!(init_fields.next_protocol(), "AES_256_CBC-HMAC_SHA256");
        let mut rng = StdRng::from_entropy();
        let peer = server().advance_state(&mut rng, init).unwrap();
        let pending = client.accept_server_init(peer.server_init_msg()).unwrap();
        let peer = peer
            .advance_state(&mut rng, pending.client_finish().unwrap())
            .unwrap();
        assert!(
            pending.auth_string().unwrap()
                == &peer
                    .completed_handshake()
                    .auth_string::<Provider>()
                    .derive_array::<32>()
                    .unwrap()
        );
        assert!(
            *pending._next_protocol_secret
                == peer
                    .completed_handshake()
                    .next_protocol_secret::<Provider>()
                    .derive_array::<32>()
                    .unwrap()
        );
        assert_eq!(
            format!("{pending:?}"),
            "PendingPhoneConfirmation { redacted }"
        );
    }

    #[test]
    fn rejects_bad_peer_messages_and_curve25519_selection() {
        for message in [&[][..], &[0xff][..], &vec![0; MESSAGE_LIMIT + 1][..]] {
            assert!(matches!(
                PairingHandshake::start().accept_server_init(message),
                Err(HandshakeError::InvalidMessage)
            ));
        }
        let client = PairingHandshake::start();
        let mut rng = StdRng::from_entropy();
        let peer = server()
            .advance_state(&mut rng, client.client_init().unwrap())
            .unwrap();
        let mut wrapper = ukey::Ukey2Message::parse_from_bytes(peer.server_init_msg()).unwrap();
        let mut fields = ukey::Ukey2ServerInit::parse_from_bytes(wrapper.message_data()).unwrap();
        fields.set_handshake_cipher(ukey::Ukey2HandshakeCipher::CURVE25519_SHA512);
        fields.set_public_key(vec![1; 32]);
        wrapper.set_message_data(fields.write_to_bytes().unwrap());
        assert!(matches!(
            client.accept_server_init(&wrapper.write_to_bytes().unwrap()),
            Err(HandshakeError::InvalidMessage)
        ));
    }

    #[test]
    fn rejects_unknown_selected_protocol_and_invalid_p256_point() {
        use ukey2_proto::ukey2_all_proto::securemessage;
        for invalid_point in [false, true] {
            let client = PairingHandshake::start();
            let mut rng = StdRng::from_entropy();
            let peer = server()
                .advance_state(&mut rng, client.client_init().unwrap())
                .unwrap();
            let mut wrapper = ukey::Ukey2Message::parse_from_bytes(peer.server_init_msg()).unwrap();
            let mut fields =
                ukey::Ukey2ServerInit::parse_from_bytes(wrapper.message_data()).unwrap();
            if invalid_point {
                let mut key =
                    securemessage::GenericPublicKey::parse_from_bytes(fields.public_key()).unwrap();
                key.ec_p256_public_key.as_mut().unwrap().set_x(vec![0; 32]);
                key.ec_p256_public_key.as_mut().unwrap().set_y(vec![0; 32]);
                fields.set_public_key(key.write_to_bytes().unwrap());
            } else {
                fields.set_selected_next_protocol("UNKNOWN_PROTOCOL".to_owned());
            }
            wrapper.set_message_data(fields.write_to_bytes().unwrap());
            assert!(matches!(
                client.accept_server_init(&wrapper.write_to_bytes().unwrap()),
                Err(HandshakeError::InvalidMessage)
            ));
        }
    }

    #[test]
    fn expiry_prevents_transcript_or_verification_access() {
        let mut client = PairingHandshake::start();
        client.started = Instant::now() - HANDSHAKE_LIFETIME;
        assert_eq!(client.client_init().unwrap_err(), HandshakeError::Expired);
        assert!(matches!(
            client.accept_server_init(&[]),
            Err(HandshakeError::Expired)
        ));
        let pending = PendingPhoneConfirmation {
            client_finish: vec![1],
            auth_string: Zeroizing::new([2; 32]),
            _next_protocol_secret: Zeroizing::new([3; 32]),
            started: Instant::now() - HANDSHAKE_LIFETIME,
        };
        assert_eq!(
            pending.client_finish().unwrap_err(),
            HandshakeError::Expired
        );
        assert_eq!(pending.auth_string().unwrap_err(), HandshakeError::Expired);
    }
}
