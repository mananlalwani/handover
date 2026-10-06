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
                media: None,
            }],
            temporary: temporary.into(),
        }),
        temporary: temporary.into(),
    };
    Ok(Zeroizing::new(request.encode_to_vec()))
}

pub(crate) fn validate_media(conversation: &str, caption: Option<&str>) -> Result<(), ProbeError> {
    identifier(conversation)?;
    if caption.is_some_and(|value| value.chars().count() > MAX_TEXT_CHARS || value.contains('\0')) {
        return Err(invalid());
    }
    Ok(())
}
pub(crate) fn media_request(
    conversation: &str,
    uploaded: &crate::media::Uploaded,
    caption: Option<&str>,
    temporary: &str,
) -> Result<Zeroizing<Vec<u8>>, ProbeError> {
    validate_media(conversation, caption)?;
    identifier(temporary)?;
    let caption = caption.filter(|value| !value.is_empty());
    let mut parts = vec![Part {
        text: None,
        media: Some(Media {
            kind: uploaded.kind,
            blob: uploaded.blob.to_string(),
            name: uploaded.name.to_string(),
            size: uploaded.size as i64,
            key: uploaded.key.to_vec(),
            mime: uploaded.mime.into(),
        }),
    }];
    if let Some(value) = caption {
        parts.push(Part {
            text: Some(Text {
                value: value.into(),
            }),
            media: None,
        });
    }
    Ok(Zeroizing::new(
        Request {
            conversation: conversation.into(),
            temporary: temporary.into(),
            message: Some(Content {
                id: temporary.into(),
                conversation: conversation.into(),
                temporary: temporary.into(),
                legacy_text: caption.map(|value| LegacyText {
                    text: Some(Text {
                        value: value.into(),
                    }),
                }),
                parts,
            }),
        }
        .encode_to_vec(),
    ))
}
/// Preserve the daemon's random 128-bit operation identity in UUID text form.
/// Other helper callers retain the existing fresh-temporary behavior.
pub(crate) fn temporary(request_id: &str) -> uuid::Uuid {
    request_id
        .strip_prefix("send-")
        .filter(|value| {
            value.len() == 32
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .unwrap_or_else(uuid::Uuid::new_v4)
}
pub(crate) fn operation_id(temporary: &str) -> Option<String> {
    if temporary.len() != 36 {
        return None;
    }
    let id = uuid::Uuid::parse_str(temporary).ok()?;
    if id.to_string() != temporary {
        return None;
    }
    Some(format!("send-{}", id.simple()))
}

pub(crate) enum Reply {
    Accepted(Option<String>),
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
            if response.message.is_empty() {
                return Ok(Reply::Accepted(None));
            }
            identifier(&response.message).map_err(|_| error(SessionError::SendReplyIdentity))?;
            Ok(Reply::Accepted(Some(std::mem::take(&mut response.message))))
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
    #[prost(message, optional, tag = "3")]
    media: Option<Media>,
}
#[derive(Message)]
#[prost(skip_debug)]
struct Media {
    #[prost(int32, tag = "1")]
    kind: i32,
    #[prost(string, tag = "2")]
    blob: String,
    #[prost(string, tag = "4")]
    name: String,
    #[prost(int64, tag = "5")]
    size: i64,
    #[prost(bytes = "vec", tag = "11")]
    key: Vec<u8>,
    #[prost(string, tag = "14")]
    mime: String,
}
impl Drop for Media {
    fn drop(&mut self) {
        self.blob.zeroize();
        self.name.zeroize();
        self.key.zeroize();
        self.mime.zeroize();
    }
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
    fn operation_identity_roundtrips_without_a_session_mapping() {
        let request = "send-0123456789abcdef0123456789abcdef";
        let wire = temporary(request).to_string();
        assert_eq!(operation_id(&wire).as_deref(), Some(request));
        for invalid in [
            "",
            "not-an-id",
            "0123456789abcdef0123456789abcdef",
            "01234567-89AB-CDEF-0123-456789ABCDEF",
        ] {
            assert!(operation_id(invalid).is_none());
        }
        assert_ne!(temporary("other-request"), temporary("other-request"));
    }
    #[test]
    fn invalid_replies_report_fixed_categories_without_payloads() {
        for (bytes, category) in [
            (&[0x12][..], SessionError::SendReplyEncoding),
            (&[][..], SessionError::MissingSendResult),
            (&[0x18, 0][..], SessionError::UnknownSendResult),
            (&[0x18, 5][..], SessionError::UnknownSendResult),
            (&[0x12, 1, 0, 0x18, 1][..], SessionError::SendReplyIdentity),
        ] {
            assert!(
                matches!(reply(bytes), Err(ProbeError::SessionProtocol(actual)) if actual == category)
            );
        }
    }
    #[test]
    fn response_requires_explicit_known_result_and_valid_optional_identity() {
        for bytes in [&[][..], &[0x18, 0], &[0x18, 5], &[0x12, 1, 0, 0x18, 1]] {
            assert!(reply(bytes).is_err());
        }
        assert!(
            matches!(reply(&[0x12,1,b'm',0x18,1]).unwrap(), Reply::Accepted(Some(id)) if id=="m")
        );
        assert!(matches!(reply(&[0x18, 1]).unwrap(), Reply::Accepted(None)));
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

    #[test]
    fn media_send_contains_original_upload_key_size_and_optional_caption() {
        let uploaded = crate::media::Uploaded {
            blob: Zeroizing::new("fixture-blob".into()),
            key: Zeroizing::new([9; 32]),
            name: Zeroizing::new("fixture.png".into()),
            mime: "image/png",
            kind: 3,
            size: 4096,
        };
        for caption in [None, Some("caption")] {
            let encoded = media_request("thread", &uploaded, caption, "temp").unwrap();
            let request = Request::decode(encoded.as_slice()).unwrap();
            let content = request.message.as_ref().unwrap();
            assert_eq!(content.id, request.temporary);
            assert_eq!(content.temporary, request.temporary);
            assert_eq!(content.conversation, "thread");
            assert_eq!(content.parts.len(), if caption.is_some() { 2 } else { 1 });
            assert!(content.parts[0].text.is_none());
            let media = content.parts[0].media.as_ref().unwrap();
            assert_eq!(media.blob, "fixture-blob");
            assert_eq!(media.key, vec![9; 32]);
            assert_eq!(media.size, 4096);
            assert_eq!(media.mime, "image/png");
            assert_eq!(media.name, "fixture.png");
            assert_eq!(
                content
                    .legacy_text
                    .as_ref()
                    .and_then(|text| text.text.as_ref())
                    .map(|text| text.value.as_str()),
                caption
            );
            if caption.is_some() {
                assert!(content.parts[1].media.is_none());
            }
        }
        assert!(media_request("", &uploaded, None, "temp").is_err());
        assert!(media_request("thread", &uploaded, Some("a\0b"), "temp").is_err());
        assert!(
            media_request(
                "thread",
                &uploaded,
                Some(&"a".repeat(MAX_TEXT_CHARS + 1)),
                "temp"
            )
            .is_err()
        );
    }
}
