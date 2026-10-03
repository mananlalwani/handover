use base64::{Engine, engine::general_purpose};
use prost::Message;
use serde_json::Value;
use std::fmt;
use zeroize::{Zeroize, Zeroizing};

use super::{ReceiveError, ReceiveRecord};
use crate::pairing::gaia::{
    AwaitingPhoneConfirmation, InitialPairing, PairingError, PhoneConfirmedPairing,
};

const ID_LIMIT: usize = 1024;
const ENVELOPE_LIMIT: usize = 32 * 1024;
const PAYLOAD_LIMIT: usize = 16 * 1024;
const ACK_LIMIT: usize = 50;

/// An inbox identifier that can only be created after a pairing reply passes
/// its request, peer, and response validation.
pub struct Acknowledgement(String);

impl fmt::Debug for Acknowledgement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Acknowledgement { redacted }")
    }
}

/// Bounded projection of the first-party ACK RPC payload. The caller owns
/// transport timing and must only submit IDs returned after successful handling.
#[derive(Default)]
pub struct AckBatch {
    ids: Vec<String>,
}

impl fmt::Debug for AckBatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AckBatch { redacted }")
    }
}

impl AckBatch {
    pub fn push(&mut self, acknowledgement: Acknowledgement) -> Result<(), ReceiveError> {
        if self.ids.len() >= ACK_LIMIT {
            return Err(ReceiveError::TooLarge);
        }
        if !self.ids.contains(&acknowledgement.0) {
            self.ids.push(acknowledgement.0);
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// kLa field 2 contains the acknowledged message IDs. This is only the
    /// protobuf payload, not an authenticated or sent HTTP request.
    pub fn payload_bytes(&self) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(
            AckPayload {
                message_ids: self.ids.clone(),
            }
            .encode_to_vec(),
        )
    }
}

/// Opaque Google reply. Parsing does not acknowledge or consume a remote inbox.
pub struct PairingReply {
    _message_id: String,
    request_id: String,
    sender: Zeroizing<Vec<u8>>,
    kind: i32,
    body: Zeroizing<Vec<u8>>,
}

impl fmt::Debug for PairingReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PairingReply { redacted }")
    }
}

impl PairingReply {
    pub fn accept_confirmation_and_ack(
        self,
        pending: AwaitingPhoneConfirmation,
    ) -> Result<(PhoneConfirmedPairing, Acknowledgement), PairingError> {
        let ack = Acknowledgement(self._message_id.clone());
        let pairing = self.accept_confirmation(pending)?;
        Ok((pairing, ack))
    }

    pub fn accept_initial_and_ack(
        self,
        attempt: InitialPairing,
    ) -> Result<(AwaitingPhoneConfirmation, Acknowledgement), PairingError> {
        let ack = Acknowledgement(self._message_id.clone());
        let pairing = self.accept_initial(attempt)?;
        Ok((pairing, ack))
    }

    pub fn accept_confirmation(
        self,
        pending: AwaitingPhoneConfirmation,
    ) -> Result<PhoneConfirmedPairing, PairingError> {
        if self.kind != 45 {
            return Err(PairingError::InvalidResponse);
        }
        pending.accept_response(&self.request_id, &self.sender, &self.body)
    }

    pub fn accept_initial(
        self,
        attempt: InitialPairing,
    ) -> Result<AwaitingPhoneConfirmation, PairingError> {
        if self.kind != 44 {
            return Err(PairingError::InvalidResponse);
        }
        attempt.accept_response(&self.request_id, &self.sender, &self.body)
    }
}

#[derive(Message)]
struct AckPayload {
    #[prost(string, repeated, tag = "2")]
    message_ids: Vec<String>,
}

impl ReceiveRecord {
    /// Extract only unencrypted pairing replies. Heartbeats and other message
    /// types return None for the eventual dispatcher; this method never ACKs them.
    pub fn pairing_reply(&self) -> Result<Option<PairingReply>, ReceiveError> {
        let fields = self.fields();
        if fields.len() > 7 {
            return Err(ReceiveError::Malformed);
        }
        let mut events = fields
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, v)| !v.is_null());
        let (index, event) = events.next().ok_or(ReceiveError::Malformed)?;
        if events.next().is_some() || !event.is_array() {
            return Err(ReceiveError::Malformed);
        }
        if index != 1 {
            return Ok(None);
        }
        let message = event.as_array().ok_or(ReceiveError::Malformed)?;
        if message.len() > 128 {
            return Err(ReceiveError::Malformed);
        }
        if message.get(1).and_then(Value::as_i64) != Some(19) {
            return Ok(None);
        }
        let bytes = decode(message.get(11), ENVELOPE_LIMIT)?;
        let header =
            ResponseHeader::decode(bytes.as_slice()).map_err(|_| ReceiveError::Malformed)?;
        if header.inactive {
            return Err(ReceiveError::SessionPreempted);
        }
        if !matches!(header.kind, 44 | 45) {
            return Ok(None);
        }
        let response =
            PairingResponse::decode(bytes.as_slice()).map_err(|_| ReceiveError::Malformed)?;
        if response.streaming
            || response.sequence < 0
            || response.sequence > 1
            || !response.encrypted.is_empty()
            || !response.additional_payload.is_empty()
            || response.body.is_empty()
            || response.body.len() > PAYLOAD_LIMIT
        {
            return Err(ReceiveError::Malformed);
        }
        let message_id = bounded_id(message.first())?;
        if response.request_id.is_empty()
            || response.request_id.len() > ID_LIMIT
            || response.request_id.chars().any(char::is_control)
        {
            return Err(ReceiveError::Malformed);
        }
        let sender = decode(message.get(16), ID_LIMIT)?;
        if sender.is_empty() {
            return Err(ReceiveError::Malformed);
        }
        Ok(Some(PairingReply {
            _message_id: message_id.to_owned(),
            request_id: response.request_id.clone(),
            sender,
            kind: response.kind,
            body: Zeroizing::new(response.body.clone()),
        }))
    }
}

// First-party hva/kva projection. Header-only decoding skips payloads for other
// request types; no chat payload decoder is present here.
#[derive(Message)]
struct ResponseHeader {
    #[prost(int32, tag = "4")]
    kind: i32,
    #[prost(bool, tag = "9")]
    inactive: bool,
}

#[derive(Message)]
struct PairingResponse {
    #[prost(string, tag = "1")]
    request_id: String,
    #[prost(int32, tag = "4")]
    kind: i32,
    #[prost(bytes = "vec", tag = "5")]
    body: Vec<u8>,
    #[prost(bool, tag = "6")]
    streaming: bool,
    #[prost(int32, tag = "7")]
    sequence: i32,
    #[prost(bytes = "vec", tag = "8")]
    encrypted: Vec<u8>,
    #[prost(bytes = "vec", tag = "11")]
    additional_payload: Vec<u8>,
}

impl Drop for PairingResponse {
    fn drop(&mut self) {
        self.body.zeroize();
        self.encrypted.zeroize();
        self.additional_payload.zeroize();
    }
}

fn bounded_id(value: Option<&Value>) -> Result<&str, ReceiveError> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= ID_LIMIT && !s.chars().any(char::is_control))
        .ok_or(ReceiveError::Malformed)
}

fn decode(value: Option<&Value>, limit: usize) -> Result<Zeroizing<Vec<u8>>, ReceiveError> {
    let encoded = value
        .and_then(Value::as_str)
        .filter(|s| s.len() <= limit.div_ceil(3) * 4)
        .ok_or(ReceiveError::Malformed)?;
    for engine in [
        general_purpose::STANDARD,
        general_purpose::STANDARD_NO_PAD,
        general_purpose::URL_SAFE,
        general_purpose::URL_SAFE_NO_PAD,
    ] {
        if let Ok(bytes) = engine.decode(encoded) {
            let bytes = Zeroizing::new(bytes);
            if bytes.len() <= limit {
                return Ok(bytes);
            }
        }
    }
    Err(ReceiveError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(response: &PairingResponse) -> ReceiveRecord {
        let mut message = vec![Value::Null; 17];
        message[0] = json!("synthetic-inbox-id");
        message[1] = json!(19);
        message[11] = json!(general_purpose::STANDARD.encode(response.encode_to_vec()));
        message[16] = json!(general_purpose::STANDARD.encode("synthetic-phone"));
        ReceiveRecord(json!([[], message]))
    }

    fn response() -> PairingResponse {
        PairingResponse {
            request_id: "synthetic-request".to_owned(),
            kind: 44,
            body: vec![1, 2],
            streaming: false,
            sequence: 0,
            encrypted: vec![],
            additional_payload: vec![],
        }
    }

    #[test]
    fn projection_retains_pairing_correlation_and_rejects_ambiguous_payloads() {
        let input = record(&response());
        let reply = input.pairing_reply().unwrap().unwrap();
        assert_eq!(reply._message_id, "synthetic-inbox-id");
        assert_eq!(reply.request_id, "synthetic-request");
        assert!(reply.sender.as_slice() == b"synthetic-phone");
        assert!(reply.body.as_slice() == [1, 2]);
        assert_eq!(format!("{reply:?}"), "PairingReply { redacted }");
        for invalid in 0..5 {
            let mut response = response();
            match invalid {
                0 => response.streaming = true,
                1 => response.sequence = 2,
                2 => response.encrypted = vec![1],
                3 => response.additional_payload = vec![1],
                _ => response.body = vec![0; PAYLOAD_LIMIT + 1],
            }
            assert!(record(&response).pairing_reply().is_err());
        }
    }

    #[test]
    fn acknowledgement_batch_is_bounded_deduplicated_and_uses_observed_field() {
        let mut batch = AckBatch::default();
        batch.push(Acknowledgement("first".to_owned())).unwrap();
        batch.push(Acknowledgement("first".to_owned())).unwrap();
        assert!(!batch.is_empty());
        let decoded = AckPayload::decode(batch.payload_bytes().as_slice()).unwrap();
        assert_eq!(decoded.message_ids, ["first"]);

        for i in 1..ACK_LIMIT {
            batch.push(Acknowledgement(format!("id-{i}"))).unwrap();
        }
        assert_eq!(
            AckPayload::decode(batch.payload_bytes().as_slice())
                .unwrap()
                .message_ids
                .len(),
            ACK_LIMIT
        );
        assert_eq!(
            batch.push(Acknowledgement("overflow".to_owned())),
            Err(ReceiveError::TooLarge)
        );
    }

    #[test]
    fn heartbeat_unrelated_replies_and_preemption_do_not_become_pairing() {
        assert!(
            ReceiveRecord(json!([[], null, []]))
                .pairing_reply()
                .unwrap()
                .is_none()
        );
        let mut response = response();
        response.kind = 16;
        response.body = vec![0xff];
        assert!(record(&response).pairing_reply().unwrap().is_none());
        let mut input = record(&response);
        let bytes = ResponseHeader {
            kind: 44,
            inactive: true,
        }
        .encode_to_vec();
        input.0[1][11] = json!(general_purpose::STANDARD.encode(bytes));
        assert!(matches!(
            input.pairing_reply(),
            Err(ReceiveError::SessionPreempted)
        ));
    }

    #[test]
    fn rejects_multiple_oneof_events_missing_sender_and_bad_binary() {
        for invalid in 0..4 {
            let mut input = record(&response());
            match invalid {
                0 => input.0.as_array_mut().unwrap().push(json!([])),
                1 => input.0[1][16] = Value::Null,
                2 => input.0[1][11] = json!("not base64!"),
                _ => input.0[1][11] = json!(general_purpose::STANDARD.encode([0xff])),
            }
            assert!(input.pairing_reply().is_err());
        }
    }

    #[test]
    fn streamed_reply_advances_only_the_correlated_upstream_handshake() {
        use crate::receive::{ReceiveEvent, ReceiveStream};
        use crate::sources::{PhoneSelection, RegisteredSources};
        use crypto_provider_rustcrypto::RustCryptoImpl;
        use rand::{SeedableRng, rngs::StdRng};
        use std::collections::HashSet;
        use ukey2_rs::{HandshakeImplementation, StateMachine, Ukey2ServerStage1};

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
        let sources = RegisteredSources::from_lookup_response(&lookup).unwrap();
        let PhoneSelection::Selected(phone) = sources.select_phone() else {
            panic!()
        };
        let attempt = InitialPairing::prepare(phone).unwrap();
        // Decode only the two relevant request fields with a separate test projection.
        #[derive(Message)]
        struct Request {
            #[prost(string, tag = "1")]
            id: String,
            #[prost(bytes = "vec", tag = "4")]
            init: Vec<u8>,
        }
        let request = Request::decode(attempt.request_bytes().unwrap()).unwrap();
        let mut rng = StdRng::from_entropy();
        let peer = Ukey2ServerStage1::<RustCryptoImpl<StdRng>>::from(
            HashSet::from(["AES_256_CBC-HMAC_SHA256".to_owned()]),
            HandshakeImplementation::PublicKeyInProtobuf,
        )
        .advance_state(&mut rng, &request.init)
        .unwrap();
        // Dza wire fields 3=true, 4=pairing ID, 5=raw server init, 6=1, 7=1.
        let mut body = vec![0x18, 1, 0x22];
        prost::encoding::encode_varint(request.id.len() as u64, &mut body);
        body.extend_from_slice(request.id.as_bytes());
        body.push(0x2a);
        prost::encoding::encode_varint(peer.server_init_msg().len() as u64, &mut body);
        body.extend_from_slice(peer.server_init_msg());
        body.extend_from_slice(&[0x30, 1, 0x38, 1]);
        let mut response = response();
        response.request_id = attempt.request_id().to_owned();
        response.body = body;
        let record = record(&response);
        let wire = serde_json::to_vec(&json!([[record.0.clone()], [0]])).unwrap();
        let mut attempt = Some(attempt);
        let mut pending = None;
        let mut acknowledgement = None;
        let mut stream = ReceiveStream::default();
        for chunk in wire.chunks(7) {
            stream
                .feed(chunk, |event| {
                    if let ReceiveEvent::Record(record) = event {
                        let reply = record.pairing_reply().unwrap().unwrap();
                        let (accepted, ack) = reply
                            .accept_initial_and_ack(attempt.take().unwrap())
                            .unwrap();
                        pending = Some(accepted);
                        acknowledgement = Some(ack);
                    }
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(stream.finish(), Ok(0));
        assert!(attempt.is_none());
        assert!(pending.unwrap().request_bytes().is_ok());
        let mut ack_batch = AckBatch::default();
        ack_batch.push(acknowledgement.unwrap()).unwrap();
        assert_eq!(
            AckPayload::decode(ack_batch.payload_bytes().as_slice())
                .unwrap()
                .message_ids,
            ["synthetic-inbox-id"]
        );
    }
}
