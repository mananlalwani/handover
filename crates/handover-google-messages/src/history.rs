//! Bounded history projection from independently observed first-party fields.
use crate::{ProbeError, session::SessionError};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use handover_core::messaging::{
    Attachment, Conversation, Message as CoreMessage, MessageId, Participant, validate_message,
};
use prost::Message;
use zeroize::{Zeroize, Zeroizing};

pub struct HistoryPage {
    pub messages: Vec<CoreMessage>,
    pub cursor_next: Option<String>,
}
impl std::fmt::Debug for HistoryPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HistoryPage { redacted }")
    }
}
fn invalid() -> ProbeError {
    ProbeError::SessionProtocol(SessionError::HistoryModel)
}

pub(crate) fn request(
    conversation: &Conversation,
    cursor: Option<&str>,
    limit: u32,
) -> Result<Zeroizing<Vec<u8>>, ProbeError> {
    handover_core::messaging::validate_conversation(conversation).map_err(|_| invalid())?;
    if !(1..=50).contains(&limit) {
        return Err(invalid());
    }
    let cursor = cursor
        .map(|value| {
            if value.len() > 2048 {
                return Err(invalid());
            }
            let bytes = Zeroizing::new(URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?);
            let cursor = Cursor::decode(bytes.as_slice()).map_err(|_| invalid())?;
            cursor.validate()?;
            Ok(cursor)
        })
        .transpose()?;
    Ok(Zeroizing::new(
        Request {
            conversation: conversation.id.local_id.clone(),
            limit: limit as i32,
            cursor,
        }
        .encode_to_vec(),
    ))
}

pub(crate) fn decode(conversation: &Conversation, bytes: &[u8]) -> Result<HistoryPage, ProbeError> {
    let mut page = Page::decode(bytes).map_err(|_| invalid())?;
    if page.messages.len() > 500 {
        return Err(invalid());
    }
    let cursor_next = page
        .cursor
        .as_ref()
        .map(|cursor| {
            cursor.validate()?;
            Ok(URL_SAFE_NO_PAD.encode(cursor.encode_to_vec()))
        })
        .transpose()?;
    let messages = decode_records(conversation, std::mem::take(&mut page.messages))?;
    Ok(HistoryPage {
        messages,
        cursor_next,
    })
}

pub(crate) fn decode_records(
    conversation: &Conversation,
    records: Vec<Vec<u8>>,
) -> Result<Vec<CoreMessage>, ProbeError> {
    // Own the raw records through a wiping container even on partial failure.
    let mut records = Zeroizing::new(records);
    if records.len() > 500 {
        return Err(invalid());
    }
    let mut messages: Vec<CoreMessage> = Vec::new();
    for bytes in records.iter_mut() {
        if bytes.len() > 64 * 1024 {
            return Err(invalid());
        }
        let mut wire = WireMessage::decode(bytes.as_slice()).map_err(|_| invalid())?;
        if wire.conversation != conversation.id.local_id {
            return Err(invalid());
        }
        // First-party Q2a treats these as non-content system entries. Keep
        // them out of ordinary message windows, without claiming a full window.
        if wire.notice
            || wire
                .status
                .as_ref()
                .is_some_and(|status| matches!(status.code, 200..=211 | 217))
        {
            continue;
        }
        if wire.parts.is_empty()
            || wire
                .status
                .as_ref()
                .is_some_and(|status| status.code <= 0 || status.code >= 200)
        {
            return Err(ProbeError::SessionProtocol(
                SessionError::UnsupportedHistoryContent,
            ));
        }
        let outgoing = wire
            .status
            .as_ref()
            .is_some_and(|status| (1..100).contains(&status.code));
        let sender = if outgoing {
            conversation
                .participants
                .iter()
                .find(|item| item.is_self)
                .cloned()
                .ok_or_else(invalid)?
        } else {
            if wire.sender.is_empty() {
                return Err(invalid());
            }
            let id = format!("peer:{}", wire.sender);
            let known = conversation
                .participants
                .iter()
                .find(|item| item.local_id == id && !item.is_self)
                .cloned();
            if wire.status.is_none() && known.is_none() {
                return Err(invalid());
            }
            known.unwrap_or(Participant {
                local_id: id,
                display_name: None,
                address: None,
                is_self: false,
            })
        };
        let mut texts = Vec::new();
        let mut attachments = Vec::new();
        for part in &mut wire.parts {
            match part.content.as_mut() {
                Some(part::Content::Text(text)) => texts.push(std::mem::take(&mut text.text)),
                Some(part::Content::Media(media)) => {
                    if part.id.is_empty() {
                        return Err(invalid());
                    }
                    attachments.push(Attachment {
                        local_id: std::mem::take(&mut part.id),
                        mime: (!media.mime.is_empty()).then(|| std::mem::take(&mut media.mime)),
                        name: (!media.name.is_empty()).then(|| std::mem::take(&mut media.name)),
                        size_bytes: media
                            .size
                            .map(u64::try_from)
                            .transpose()
                            .map_err(|_| invalid())?,
                        staged_path: None,
                    });
                }
                None => {
                    return Err(ProbeError::SessionProtocol(
                        SessionError::UnsupportedHistoryContent,
                    ));
                }
            }
        }
        let text = (!texts.is_empty()).then(|| texts.join("\n"));
        texts.iter_mut().for_each(Zeroize::zeroize);
        let record = CoreMessage {
            id: MessageId::new(conversation.id.clone(), std::mem::take(&mut wire.id)),
            sender,
            transport: None,
            sent_at: wire.timestamp.filter(|value| *value > 0),
            text,
            attachments,
            reply_to: None,
            reactions: Vec::new(),
            deleted: false,
        };
        validate_message(&record).map_err(|_| invalid())?;
        if let Some(previous) = messages.iter().find(|previous| previous.id == record.id) {
            if previous != &record {
                return Err(invalid());
            }
        } else {
            messages.push(record);
        }
    }
    Ok(messages)
}

#[derive(Message)]
#[prost(skip_debug)]
struct Request {
    #[prost(string, tag = "2")]
    conversation: String,
    #[prost(int32, tag = "3")]
    limit: i32,
    #[prost(message, optional, tag = "5")]
    cursor: Option<Cursor>,
}
impl Drop for Request {
    fn drop(&mut self) {
        self.conversation.zeroize();
    }
}
#[derive(Message)]
#[prost(skip_debug)]
struct Cursor {
    #[prost(string, tag = "1")]
    id: String,
    #[prost(int64, tag = "2")]
    timestamp: i64,
}
impl Cursor {
    fn validate(&self) -> Result<(), ProbeError> {
        if self.id.is_empty()
            || self.id.len() > 1024
            || self.id.chars().any(char::is_control)
            || self.timestamp < 0
        {
            return Err(invalid());
        }
        Ok(())
    }
}
impl Drop for Cursor {
    fn drop(&mut self) {
        self.id.zeroize();
    }
}
#[derive(Message)]
#[prost(skip_debug)]
struct Page {
    #[prost(bytes = "vec", repeated, tag = "2")]
    messages: Vec<Vec<u8>>,
    #[prost(message, optional, tag = "5")]
    cursor: Option<Cursor>,
}
impl Drop for Page {
    fn drop(&mut self) {
        self.messages.iter_mut().for_each(Zeroize::zeroize);
    }
}
#[derive(Message)]
#[prost(skip_debug)]
struct WireMessage {
    #[prost(string, tag = "1")]
    id: String,
    #[prost(message, optional, tag = "4")]
    status: Option<Status>,
    #[prost(int64, optional, tag = "5")]
    timestamp: Option<i64>,
    #[prost(string, tag = "7")]
    conversation: String,
    #[prost(string, tag = "9")]
    sender: String,
    #[prost(message, repeated, tag = "10")]
    parts: Vec<Part>,
    #[prost(bool, tag = "16")]
    notice: bool,
}
impl Drop for WireMessage {
    fn drop(&mut self) {
        self.id.zeroize();
        self.conversation.zeroize();
        self.sender.zeroize();
    }
}
#[derive(Message)]
struct Status {
    #[prost(int32, tag = "2")]
    code: i32,
}
#[derive(Message)]
#[prost(skip_debug)]
struct Part {
    #[prost(string, tag = "1")]
    id: String,
    #[prost(oneof = "part::Content", tags = "2,3")]
    content: Option<part::Content>,
}
impl Drop for Part {
    fn drop(&mut self) {
        self.id.zeroize();
    }
}
mod part {
    #[derive(prost::Oneof)]
    #[prost(skip_debug)]
    pub enum Content {
        #[prost(message, tag = "2")]
        Text(super::Text),
        #[prost(message, tag = "3")]
        Media(super::Media),
    }
}
#[derive(Message)]
#[prost(skip_debug)]
struct Text {
    #[prost(string, tag = "1")]
    text: String,
}
impl Drop for Text {
    fn drop(&mut self) {
        self.text.zeroize();
    }
}
#[derive(Message)]
#[prost(skip_debug)]
struct Media {
    #[prost(string, tag = "4")]
    name: String,
    #[prost(int64, optional, tag = "5")]
    size: Option<i64>,
    #[prost(string, tag = "14")]
    mime: String,
}
impl Drop for Media {
    fn drop(&mut self) {
        self.name.zeroize();
        self.mime.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use handover_core::messaging::{
        ConversationId, ConversationKind, MessagingAccountId, TransportKind,
    };
    fn conversation() -> Conversation {
        Conversation {
            id: ConversationId::new(MessagingAccountId::new("fixture"), "thread"),
            kind: ConversationKind::Direct,
            transport: TransportKind::Unknown,
            title: None,
            participants: vec![
                Participant {
                    local_id: "self:fixture".into(),
                    display_name: None,
                    address: None,
                    is_self: true,
                },
                Participant {
                    local_id: "peer:person".into(),
                    display_name: Some("Fixture".into()),
                    address: None,
                    is_self: false,
                },
            ],
            latest_message_id: None,
            last_activity_at: None,
            unread_count: None,
            cursor: None,
            capabilities: Default::default(),
        }
    }
    fn record() -> WireMessage {
        WireMessage {
            id: "message".into(),
            status: Some(Status { code: 100 }),
            timestamp: Some(1234000),
            conversation: "thread".into(),
            sender: "person".into(),
            parts: vec![Part {
                id: "part".into(),
                content: Some(part::Content::Text(Text {
                    text: "Synthetic content".into(),
                })),
            }],
            notice: false,
        }
    }
    fn page(record: WireMessage) -> Vec<u8> {
        Page {
            messages: vec![record.encode_to_vec()],
            cursor: None,
        }
        .encode_to_vec()
    }
    #[test]
    fn projects_text_and_sender_without_inventing_transport_or_status() {
        let decoded = decode(&conversation(), &page(record())).unwrap();
        assert_eq!(
            decoded.messages[0].text.as_deref(),
            Some("Synthetic content")
        );
        assert_eq!(decoded.messages[0].sender, conversation().participants[1]);
        assert_eq!(decoded.messages[0].sent_at, Some(1234000));
        assert_eq!(decoded.messages[0].transport, None);
        assert_eq!(format!("{decoded:?}"), "HistoryPage { redacted }");
        let mut sent = record();
        sent.status = Some(Status { code: 1 });
        assert!(
            decode(&conversation(), &page(sent)).unwrap().messages[0]
                .sender
                .is_self
        );
    }
    #[test]
    fn binds_records_to_the_requested_thread_and_rejects_unsupported_content() {
        let mut wrong = record();
        wrong.conversation = "another".into();
        assert!(decode(&conversation(), &page(wrong)).is_err());
        let mut unsupported = record();
        unsupported.parts[0].content = None;
        assert!(matches!(
            decode(&conversation(), &page(unsupported)),
            Err(ProbeError::SessionProtocol(
                SessionError::UnsupportedHistoryContent
            ))
        ));
    }
    #[test]
    fn preserves_media_metadata_without_download_or_staged_path() {
        let mut media = record();
        media.parts[0].content = Some(part::Content::Media(Media {
            name: "fixture.png".into(),
            mime: "image/png".into(),
            size: Some(321),
        }));
        let decoded = decode(&conversation(), &page(media)).unwrap();
        assert_eq!(
            decoded.messages[0].attachments[0].mime.as_deref(),
            Some("image/png")
        );
        assert_eq!(decoded.messages[0].attachments[0].size_bytes, Some(321));
        assert_eq!(decoded.messages[0].attachments[0].staged_path, None);
    }
    #[test]
    fn opaque_cursor_roundtrips_through_the_next_request_and_bounds_input() {
        let bytes = Page {
            messages: Vec::new(),
            cursor: Some(Cursor {
                id: "older".into(),
                timestamp: 7,
            }),
        }
        .encode_to_vec();
        let decoded = decode(&conversation(), &bytes).unwrap();
        let encoded = request(&conversation(), decoded.cursor_next.as_deref(), 25).unwrap();
        let next = Request::decode(encoded.as_slice()).unwrap();
        assert_eq!(next.cursor.as_ref().unwrap().id, "older");
        assert!(request(&conversation(), Some(&"A".repeat(2049)), 25).is_err());
        assert!(request(&conversation(), None, 0).is_err());
        assert!(request(&conversation(), None, 51).is_err());
    }
}
