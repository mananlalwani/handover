//! Correlated encrypted RPC replies. No acknowledgement is created by parsing.
use super::{ReceiveError, ReceiveRecord};
use base64::{Engine, engine::general_purpose::STANDARD};
use prost::Message;
use zeroize::{Zeroize, Zeroizing};

pub(crate) struct SessionReply {
    pub(crate) ciphertext: Zeroizing<Vec<u8>>,
    pub(crate) message_id: Zeroizing<String>,
    pub(crate) request_id: Zeroizing<String>,
}
#[derive(Message)]
struct Response {
    #[prost(string, tag = "1")]
    request_id: String,
    #[prost(int32, tag = "4")]
    action: i32,
    #[prost(bytes = "vec", tag = "5")]
    plaintext: Vec<u8>,
    #[prost(bytes = "vec", tag = "8")]
    encrypted: Vec<u8>,
    #[prost(bool, tag = "9")]
    inactive: bool,
    #[prost(bytes = "vec", tag = "11")]
    auxiliary: Vec<u8>,
}
impl Drop for Response {
    fn drop(&mut self) {
        self.request_id.zeroize();
        self.plaintext.zeroize();
        self.encrypted.zeroize();
        self.auxiliary.zeroize();
    }
}
impl ReceiveRecord {
    pub(crate) fn session_reply(
        &self,
        request_id: &str,
        action: i32,
        peer: &[u8],
    ) -> Result<Option<SessionReply>, ReceiveError> {
        let fields = self.fields();
        if fields.len() > 7 {
            return Err(ReceiveError::InvalidEvents);
        }
        let mut events = fields
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, value)| !value.is_null());
        let Some((index, event)) = events.next() else {
            return Ok(None);
        };
        if events.next().is_some() || !event.is_array() {
            return Err(ReceiveError::InvalidEvents);
        }
        if index != 1 {
            return Ok(None);
        }
        let message = event.as_array().ok_or(ReceiveError::InvalidEnvelope)?;
        if message.len() > 128 {
            return Err(ReceiveError::InvalidEnvelope);
        }
        if message.get(1).and_then(serde_json::Value::as_i64) != Some(19) {
            return Ok(None);
        }
        let encoded = message
            .get(11)
            .and_then(serde_json::Value::as_str)
            .ok_or(ReceiveError::InvalidEnvelope)?;
        if encoded.len() > 700 * 1024 {
            return Err(ReceiveError::TooLarge);
        }
        let bytes = Zeroizing::new(
            STANDARD
                .decode(encoded)
                .map_err(|_| ReceiveError::InvalidEnvelope)?,
        );
        let response =
            Response::decode(bytes.as_slice()).map_err(|_| ReceiveError::InvalidEnvelope)?;
        if response.request_id != request_id || response.action != action {
            return Ok(None);
        }
        let sender = message
            .get(16)
            .and_then(serde_json::Value::as_str)
            .ok_or(ReceiveError::InvalidIdentifiers)?;
        if sender.len() > 1400 {
            return Err(ReceiveError::InvalidIdentifiers);
        }
        let sender = Zeroizing::new(
            STANDARD
                .decode(sender)
                .map_err(|_| ReceiveError::InvalidIdentifiers)?,
        );
        if sender.as_slice() != peer {
            return Ok(None);
        }
        if response.inactive {
            return Err(ReceiveError::SessionPreempted);
        }
        // Observed action 10 completes without a body. It attests no data or
        // read effect; callers must refresh encrypted inventory afterward.
        let empty_read_completion = action == 10 && response.encrypted.is_empty();
        if !response.plaintext.is_empty()
            || !response.auxiliary.is_empty()
            || (!empty_read_completion && response.encrypted.len() < 48)
            || response.encrypted.len() > 512 * 1024 + 48
        {
            return Err(ReceiveError::InvalidEnvelope);
        }
        let id = message
            .first()
            .and_then(serde_json::Value::as_str)
            .ok_or(ReceiveError::InvalidIdentifiers)?;
        if id.is_empty() || id.len() > 1024 || id.chars().any(char::is_control) {
            return Err(ReceiveError::InvalidIdentifiers);
        }
        Ok(Some(SessionReply {
            ciphertext: Zeroizing::new(response.encrypted.clone()),
            message_id: Zeroizing::new(id.to_owned()),
            request_id: Zeroizing::new(response.request_id.clone()),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    fn record(id: &str, peer: &str, inactive: bool, plaintext: Vec<u8>) -> ReceiveRecord {
        let response = Response {
            request_id: id.into(),
            action: 1,
            plaintext,
            encrypted: vec![7; 48],
            inactive,
            auxiliary: Vec::new(),
        };
        let mut message = vec![Value::Null; 17];
        message[0] = json!("inbox-id");
        message[1] = json!(19);
        message[11] = json!(STANDARD.encode(response.encode_to_vec()));
        message[16] = json!(STANDARD.encode(peer));
        ReceiveRecord(json!([[], message]))
    }
    #[test]
    fn empty_mark_read_completion_is_not_a_data_reply() {
        for action in [1, 10, 31, 38, 46] {
            let mut record = record("request", "phone", false, Vec::new());
            let response = Response {
                request_id: "request".into(),
                action,
                plaintext: vec![],
                encrypted: vec![],
                inactive: false,
                auxiliary: vec![],
            };
            record.0[1][11] = json!(STANDARD.encode(response.encode_to_vec()));
            let result = record.session_reply("request", action, b"phone");
            if action == 10 {
                assert!(
                    result.unwrap().unwrap().ciphertext.is_empty(),
                    "observed empty mark-read completion is not confirmed read state"
                );
                assert!(
                    record
                        .session_reply("other", 10, b"phone")
                        .unwrap()
                        .is_none()
                );
                assert!(
                    record
                        .session_reply("request", 10, b"other")
                        .unwrap()
                        .is_none()
                );
            } else {
                assert!(matches!(result, Err(ReceiveError::InvalidEnvelope)));
            }
        }
    }
    #[test]
    fn correlation_precedes_payload_and_preemption_validation() {
        assert!(
            record("other", "phone", true, vec![1])
                .session_reply("request", 1, b"phone")
                .unwrap()
                .is_none()
        );
        assert!(
            record("request", "other-phone", true, vec![1])
                .session_reply("request", 1, b"phone")
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            record("request", "phone", true, Vec::new()).session_reply("request", 1, b"phone"),
            Err(ReceiveError::SessionPreempted)
        ));
        assert!(matches!(
            record("request", "phone", false, vec![1]).session_reply("request", 1, b"phone"),
            Err(ReceiveError::InvalidEnvelope)
        ));
        assert!(
            record("request", "phone", false, Vec::new())
                .session_reply("request", 1, b"phone")
                .unwrap()
                .is_some()
        );
    }
}
