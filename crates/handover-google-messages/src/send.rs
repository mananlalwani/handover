//! Minimal text-send fields observed in the first-party web client.
use crate::{ProbeError, session::SessionError};
use handover_core::messaging::{MAX_ID_LEN, MAX_TEXT_CHARS};
use prost::Message;
use zeroize::{Zeroize, Zeroizing};

fn identifier(value: &str) -> Result<(), ProbeError> {
    if value.is_empty() || value.len() > MAX_ID_LEN || value.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(())
}
fn invalid() -> ProbeError {
    ProbeError::SessionProtocol(SessionError::SendModel)
}
/// All identifiers here are private protocol identifiers, never account secrets.
pub(crate) fn request(
    conversation: &str,
    text: &str,
    temporary: &str,
) -> Result<Zeroizing<Vec<u8>>, ProbeError> {
    identifier(conversation)?;
    identifier(temporary)?;
    if text.trim().is_empty() || text.chars().count() > MAX_TEXT_CHARS || text.contains('\0') {
        return Err(invalid());
    }
    let request = Request {
        conversation: conversation.into(),
        message: Some(Content {
            id: temporary.into(),
            legacy_text: Some(LegacyText {
                text: Some(Text { value: text.into() }),
            }),
            conversation: conversation.into(),
            parts: vec![Part {
                text: Some(Text { value: text.into() }),
            }],
            temporary: temporary.into(),
        }),
        temporary: temporary.into(),
    };
    Ok(Zeroizing::new(request.encode_to_vec()))
}
pub(crate) enum Reply {
    Accepted(String),
    Rejected,
}
pub(crate) fn reply(bytes: &[u8]) -> Result<Reply, ProbeError> {
    let error = |category| ProbeError::SessionProtocol(category);
    if bytes.len() > 64 * 1024 {
        return Err(error(SessionError::SendReplyEncoding));
    }
    let mut response =
        Response::decode(bytes).map_err(|_| error(SessionError::SendReplyEncoding))?;
    match response.result {
        Some(1) => {
            identifier(&response.message).map_err(|_| error(SessionError::SendReplyIdentity))?;
            Ok(Reply::Accepted(std::mem::take(&mut response.message)))
        }
        Some(2..=4) => Ok(Reply::Rejected),
        None => Err(error(SessionError::MissingSendResult)),
        Some(_) => Err(error(SessionError::UnknownSendResult)),
    }
}
pub(crate) fn capability(bytes: &[u8]) -> Result<bool, ProbeError> {
    if bytes.len() > 4096 {
        return Err(invalid());
    }
    Capability::decode(bytes)
        .map_err(|_| invalid())
        .map(|value| value.allowed)
}
#[derive(Message)]
#[prost(skip_debug)]
struct Request {
    #[prost(string, tag = "2")]
    conversation: String,
    #[prost(message, optional, tag = "3")]
    message: Option<Content>,
    #[prost(string, tag = "5")]
    temporary: String,
}
#[derive(Message)]
#[prost(skip_debug)]
struct Content {
    #[prost(string, tag = "1")]
    id: String,
    #[prost(message, optional, tag = "6")]
    legacy_text: Option<LegacyText>,
    #[prost(string, tag = "7")]
    conversation: String,
    #[prost(message, repeated, tag = "10")]
    parts: Vec<Part>,
    #[prost(string, tag = "12")]
    temporary: String,
}
#[derive(Message)]
#[prost(skip_debug)]
struct LegacyText {
    #[prost(message, optional, tag = "1")]
    text: Option<Text>,
}
#[derive(Message)]
#[prost(skip_debug)]
struct Part {
    #[prost(message, optional, tag = "2")]
    text: Option<Text>,
}
#[derive(Message)]
#[prost(skip_debug)]
struct Text {
    #[prost(string, tag = "1")]
    value: String,
}
#[derive(Message)]
#[prost(skip_debug)]
struct Response {
    #[prost(string, tag = "2")]
    message: String,
    #[prost(int32, optional, tag = "3")]
    result: Option<i32>,
}
#[derive(Message)]
struct Capability {
    #[prost(bool, tag = "1")]
    allowed: bool,
}
impl Drop for Text {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}
impl Drop for Content {
    fn drop(&mut self) {
        self.id.zeroize();
        self.conversation.zeroize();
        self.temporary.zeroize();
    }
}
impl Drop for Request {
    fn drop(&mut self) {
        self.conversation.zeroize();
        self.temporary.zeroize();
    }
}
impl Drop for Response {
    fn drop(&mut self) {
        self.message.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_replies_report_fixed_categories_without_payloads() {
        for (bytes, category) in [
            (&[0x12][..], SessionError::SendReplyEncoding),
            (&[][..], SessionError::MissingSendResult),
            (&[0x18, 0][..], SessionError::UnknownSendResult),
            (&[0x18, 5][..], SessionError::UnknownSendResult),
            (&[0x18, 1][..], SessionError::SendReplyIdentity),
        ] {
            assert!(
                matches!(reply(bytes), Err(ProbeError::SessionProtocol(actual)) if actual == category)
            );
        }
    }
    #[test]
    fn response_requires_explicit_known_result_and_success_identity() {
        for bytes in [
            &[][..],
            &[0x18, 0],
            &[0x18, 5],
            &[0x18, 1],
            &[0x12, 1, 0, 0x18, 1],
        ] {
            assert!(reply(bytes).is_err());
        }
        assert!(matches!(reply(&[0x12,1,b'm',0x18,1]).unwrap(), Reply::Accepted(id) if id=="m"));
        for code in 2..=4 {
            assert!(matches!(reply(&[0x18, code]).unwrap(), Reply::Rejected));
        }
    }
    #[test]
    fn request_rejects_invalid_text_and_identifiers() {
        assert!(request("thread", " ", "temp").is_err());
        assert!(request("thread", "a\0b", "temp").is_err());
        assert!(request("", "hello", "temp").is_err());
        assert!(request("thread", &"a".repeat(MAX_TEXT_CHARS + 1), "temp").is_err());
    }
    #[test]
    fn text_is_present_in_both_first_party_content_encodings() {
        let encoded = request("thread", "hello", "temp").unwrap();
        let request = Request::decode(encoded.as_slice()).unwrap();
        assert_eq!(request.conversation, "thread");
        let content = request.message.as_ref().unwrap();
        assert_eq!(content.id, request.temporary);
        assert_eq!(content.temporary, request.temporary);
        assert_eq!(content.parts[0].text.as_ref().unwrap().value, "hello");
        assert_eq!(
            content
                .legacy_text
                .as_ref()
                .unwrap()
                .text
                .as_ref()
                .unwrap()
                .value,
            "hello"
        );
        assert!(!capability(&[]).unwrap());
        assert!(capability(&[8, 1]).unwrap());
    }
}
