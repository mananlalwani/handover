//! Independent projection of encrypted first-party push updates.
use crate::{ProbeError, session::SessionError};
use handover_core::messaging::{Conversation, Message as CoreMessage, MessageId};
use prost::Message;
use zeroize::{Zeroize, Zeroizing};

/// A bounded observer result. It does not attest account connectivity.
pub enum Update {
    Conversations(Vec<Conversation>),
    Messages {
        records: Vec<CoreMessage>,
        statuses: Vec<(MessageId, &'static str)>,
        correlations: Vec<(String, MessageId)>,
    },
    Active,
    Inactive,
    PresenceCheck,
    Unsupported(u8),
}
impl std::fmt::Debug for Update {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Update { redacted }")
    }
}
fn invalid() -> ProbeError {
    ProbeError::SessionProtocol(SessionError::UpdateModel)
}

pub(crate) fn decode(
    account: &str,
    known: &[Conversation],
    bytes: &[u8],
    own_session: bool,
) -> Result<Update, ProbeError> {
    if bytes.len() > 512 * 1024 {
        return Err(invalid());
    }
    let fields = fields(bytes)?;
    // yw/xw is a oneof across fields 2..15. Reject ambiguity instead of
    // accepting prost's last-wins interpretation of conflicting event types.
    let mut variants = fields.iter().filter(|(tag, _)| (2..=15).contains(tag));
    let (tag, payload) = variants.next().ok_or_else(invalid)?;
    if variants.next().is_some() {
        return Err(invalid());
    }
    match *tag {
        2 => {
            let batch = Batch::decode(*payload).map_err(|_| invalid())?;
            if batch.records.len() > 1024 {
                return Err(invalid());
            }
            let mut records = Vec::<Conversation>::new();
            for bytes in &batch.records {
                if bytes.is_empty() || bytes.len() > 64 * 1024 {
                    return Err(invalid());
                }
                let record = crate::conversation::decode(account, bytes)?;
                if let Some(previous) = records.iter().find(|item| item.id == record.id) {
                    if previous != &record {
                        return Err(invalid());
                    }
                } else {
                    records.push(record);
                }
            }
            Ok(Update::Conversations(records))
        }
        3 => {
            let mut batch = Batch::decode(*payload).map_err(|_| invalid())?;
            if batch.records.len() > 500 {
                return Err(invalid());
            }
            let mut messages = Vec::<CoreMessage>::new();
            let mut statuses = Vec::<(MessageId, &'static str)>::new();
            let mut correlations = Vec::<(String, MessageId)>::new();
            let raw = Zeroizing::new(std::mem::take(&mut batch.records));
            for bytes in raw.iter() {
                if bytes.len() > 64 * 1024 {
                    return Err(invalid());
                }
                let thread = Thread::decode(bytes.as_slice()).map_err(|_| invalid())?;
                let conversation = known
                    .iter()
                    .find(|item| {
                        item.id.account_id.as_str() == account && item.id.local_id == thread.id
                    })
                    .ok_or(ProbeError::SessionProtocol(
                        SessionError::UnknownUpdateConversation,
                    ))?;
                let records =
                    match crate::history::decode_records(conversation, vec![bytes.to_vec()]) {
                        Ok(records) => records,
                        // This authenticated batch cannot be projected completely.
                        // Leave the whole update in the inbox, like other unsupported
                        // families, without discarding it or terminating receive.
                        Err(ProbeError::SessionProtocol(
                            SessionError::UnsupportedHistoryContent,
                        )) => {
                            return Ok(Update::Unsupported(3));
                        }
                        Err(error) => return Err(error),
                    };
                for record in records {
                    let outgoing = record.sender.is_self;
                    let id = record.id.clone();
                    if let Some(previous) = messages.iter().find(|item| item.id == record.id) {
                        if previous != &record {
                            return Err(invalid());
                        }
                    } else {
                        messages.push(record);
                    }
                    let lifecycle = thread.status.as_ref().and_then(|value| status(value.code));
                    if outgoing && lifecycle.is_some() {
                        if let Some(request_id) = crate::send::operation_id(&thread.temporary) {
                            if let Some((previous_request, previous)) =
                                correlations.iter().find(|(request, previous)| {
                                    *request == request_id || *previous == id
                                })
                            {
                                if *previous != id || *previous_request != request_id {
                                    return Err(invalid());
                                }
                            } else {
                                correlations.push((request_id, id.clone()));
                            }
                        }
                        if let Some(status) = lifecycle {
                            if let Some((_, previous)) =
                                statuses.iter().find(|(previous_id, _)| *previous_id == id)
                            {
                                if *previous != status {
                                    return Err(invalid());
                                }
                            } else {
                                statuses.push((id, status));
                            }
                        }
                    }
                }
            }
            Ok(Update::Messages {
                records: messages,
                statuses,
                correlations,
            })
        }
        6 => {
            let alert = Alert::decode(*payload).map_err(|_| invalid())?;
            match alert.kind {
                2 if own_session => Ok(Update::Active),
                1 | 7 | 8 if own_session => Ok(Update::Inactive),
                _ => Ok(Update::Unsupported(6)),
            }
        }
        7 => {
            let presence = Presence::decode(*payload).map_err(|_| invalid())?;
            if presence.id.len() > 1024 || presence.id.chars().any(char::is_control) {
                return Err(invalid());
            }
            Ok(Update::PresenceCheck)
        }
        other => Ok(Update::Unsupported(other as u8)),
    }
}

// Scan wire fields without decoding unknown bodies or silently discarding
// duplicate oneof members. This is ordinary protobuf framing, not cryptography.
fn fields(mut bytes: &[u8]) -> Result<Vec<(u32, &[u8])>, ProbeError> {
    let mut fields = Vec::new();
    let mut count = 0;
    while !bytes.is_empty() {
        count += 1;
        if count > 64 {
            return Err(invalid());
        }
        let key = prost::encoding::decode_varint(&mut bytes).map_err(|_| invalid())?;
        let tag = u32::try_from(key >> 3).map_err(|_| invalid())?;
        if tag == 0 || tag > 0x1fff_ffff {
            return Err(invalid());
        }
        if (2..=15).contains(&tag) && key & 7 != 2 {
            return Err(invalid());
        }
        match key & 7 {
            0 => {
                prost::encoding::decode_varint(&mut bytes).map_err(|_| invalid())?;
            }
            1 | 5 => {
                let count = if key & 7 == 1 { 8 } else { 4 };
                bytes = bytes.get(count..).ok_or_else(invalid)?;
            }
            2 => {
                let length = usize::try_from(
                    prost::encoding::decode_varint(&mut bytes).map_err(|_| invalid())?,
                )
                .map_err(|_| invalid())?;
                let value = bytes.get(..length).ok_or_else(invalid)?;
                fields.push((tag, value));
                bytes = &bytes[length..];
            }
            _ => return Err(invalid()),
        }
    }
    Ok(fields)
}
#[derive(Message)]
#[prost(skip_debug)]
struct Batch {
    #[prost(bytes = "vec", repeated, tag = "2")]
    records: Vec<Vec<u8>>,
}
impl Drop for Batch {
    fn drop(&mut self) {
        self.records.iter_mut().for_each(Zeroize::zeroize);
    }
}
#[derive(Message)]
#[prost(skip_debug)]
struct Thread {
    #[prost(string, tag = "7")]
    id: String,
    #[prost(message, optional, tag = "4")]
    status: Option<Status>,
    #[prost(string, tag = "12")]
    temporary: String,
}
#[derive(Message)]
struct Status {
    #[prost(int32, tag = "2")]
    code: i32,
}
// Q2a/M2a in the first-party client distinguish these terminal outgoing states.
// Other codes carry no supported delivery assertion.
fn status(code: i32) -> Option<&'static str> {
    match code {
        1 => Some("sent"),
        2 => Some("delivered"),
        11 => Some("displayed"),
        _ => None,
    }
}
impl Drop for Thread {
    fn drop(&mut self) {
        self.id.zeroize();
        self.temporary.zeroize();
    }
}
#[derive(Message)]
#[prost(skip_debug)]
struct Presence {
    #[prost(string, tag = "1")]
    id: String,
}
impl Drop for Presence {
    fn drop(&mut self) {
        self.id.zeroize();
    }
}
#[derive(Message)]
struct Alert {
    #[prost(int32, tag = "2")]
    kind: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(code: i32) -> (Conversation, Vec<u8>) {
        use handover_core::messaging::{
            ConversationId, ConversationKind, MessagingAccountId, Participant, TransportKind,
        };
        let conversation = Conversation {
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
                    display_name: None,
                    address: None,
                    is_self: false,
                },
            ],
            latest_message_id: None,
            last_activity_at: None,
            unread_count: None,
            cursor: None,
            capabilities: Default::default(),
        };
        let mut message = vec![0x0a, 1, b'm', 0x22, 2, 16, code as u8, 0x3a, 6];
        message.extend_from_slice(b"thread");
        message.extend_from_slice(&[0x4a, 6]);
        message.extend_from_slice(b"person");
        message.extend_from_slice(&[0x52, 5, 0x12, 3, 0x0a, 1, b'x']);
        (conversation, message)
    }
    fn push(records: Vec<Vec<u8>>) -> Vec<u8> {
        let payload = Batch { records }.encode_to_vec();
        let mut push = vec![0x1a];
        prost::encoding::encode_varint(payload.len() as u64, &mut push);
        push.extend_from_slice(&payload);
        push
    }
    #[test]
    fn publishes_only_explicit_outgoing_delivery_states_bound_to_valid_messages() {
        for (code, expected) in [
            (1, Some("sent")),
            (2, Some("delivered")),
            (11, Some("displayed")),
            (3, None),
            (100, None),
        ] {
            let (conversation, record) = fixture(code);
            let Update::Messages {
                records, statuses, ..
            } = decode(
                "fixture",
                std::slice::from_ref(&conversation),
                &push(vec![record]),
                true,
            )
            .unwrap()
            else {
                panic!("messages")
            };
            assert_eq!(records.len(), 1);
            assert_eq!(statuses.len(), usize::from(expected.is_some()));
            if let Some(expected) = expected {
                assert_eq!(statuses[0], (records[0].id.clone(), expected));
                assert!(records[0].sender.is_self);
                assert_eq!(records[0].id.conversation_id, conversation.id);
            }
        }
        let (conversation, first) = fixture(1);
        let (_, second) = fixture(2);
        assert!(decode("fixture", &[conversation], &push(vec![first, second]), true).is_err());
    }
    #[test]
    fn temporary_identity_links_only_attested_outgoing_states() {
        let operation = "send-0123456789abcdef0123456789abcdef";
        let temporary = crate::send::temporary(operation).to_string();
        for code in [1, 2, 11, 3, 100] {
            let (conversation, mut record) = fixture(code);
            record.extend_from_slice(&[0x62, 36]);
            record.extend_from_slice(temporary.as_bytes());
            let Update::Messages {
                records,
                correlations,
                ..
            } = decode("fixture", &[conversation], &push(vec![record]), true).unwrap()
            else {
                panic!("messages")
            };
            if [1, 2, 11].contains(&code) {
                assert_eq!(
                    correlations,
                    vec![(operation.into(), records[0].id.clone())]
                );
            } else {
                assert!(correlations.is_empty());
            }
        }
        let (conversation, mut first) = fixture(1);
        first.extend_from_slice(&[0x62, 36]);
        first.extend_from_slice(temporary.as_bytes());
        let mut second = first.clone();
        second[2] = b'n';
        assert!(
            decode(
                "fixture",
                std::slice::from_ref(&conversation),
                &push(vec![first.clone(), second]),
                true
            )
            .is_err()
        );
        let mut second_operation = first.clone();
        *second_operation.last_mut().unwrap() = b'e';
        assert!(
            decode(
                "fixture",
                &[conversation],
                &push(vec![first, second_operation]),
                true
            )
            .is_err()
        );
    }
    #[test]
    fn unsupported_content_leaves_the_whole_batch_unprojected() {
        let (conversation, supported) = fixture(100);
        let mut unsupported = supported.clone();
        // Remove the final text part, retaining valid identity and thread fields.
        unsupported.truncate(unsupported.len() - 7);
        let mut unknown_part = unsupported.clone();
        unknown_part.extend_from_slice(&[0x52, 0]);
        let (_, unknown_status) = fixture(0);
        for records in [
            vec![unsupported.clone()],
            vec![unknown_part],
            vec![unknown_status],
            vec![supported.clone(), unsupported],
        ] {
            assert!(matches!(
                decode(
                    "fixture",
                    std::slice::from_ref(&conversation),
                    &push(records),
                    true
                )
                .unwrap(),
                Update::Unsupported(3)
            ));
        }
        let mut malformed = supported;
        malformed.pop();
        assert!(decode("fixture", &[conversation], &push(vec![malformed]), true).is_err());
    }
    #[test]
    fn session_control_requires_the_current_session() {
        assert!(matches!(
            decode("fixture", &[], &[0x32, 2, 0x10, 2], true).unwrap(),
            Update::Active
        ));
        assert!(matches!(
            decode("fixture", &[], &[0x32, 2, 0x10, 2], false).unwrap(),
            Update::Unsupported(6)
        ));
        assert!(matches!(
            decode("fixture", &[], &[0x32, 2, 0x10, 1], true).unwrap(),
            Update::Inactive
        ));
        assert!(matches!(
            decode("fixture", &[], &[0x3a, 0], true).unwrap(),
            Update::PresenceCheck
        ));
        assert_eq!(format!("{:?}", Update::Active), "Update { redacted }");
    }
    #[test]
    fn presence_checks_accept_bounded_identifiers_and_reject_malformed_text() {
        assert!(matches!(
            decode("fixture", &[], &[0x3a, 3, 0x0a, 1, b'x'], true).unwrap(),
            Update::PresenceCheck
        ));
        assert!(decode("fixture", &[], &[0x3a, 3, 0x0a, 1, 0xff], true).is_err());
    }
    #[test]
    fn ambiguous_truncated_and_unknown_updates_do_not_become_messages() {
        for bytes in [
            &[0x32, 2, 0x10, 2, 0x3a, 0][..],
            &[0x32, 2, 0x10][..],
            &[0x00][..],
            &[0x32, 0, 0x32, 0][..],
        ] {
            assert!(decode("fixture", &[], bytes, true).is_err());
        }
        assert!(matches!(
            decode("fixture", &[], &[0x2a, 0], true).unwrap(),
            Update::Unsupported(5)
        ));
    }
    #[test]
    fn message_updates_require_a_known_thread() {
        // yw field 3 contains uw field 2, whose sw thread is field 7.
        assert!(matches!(
            decode("fixture", &[], &[0x1a, 5, 0x12, 3, 0x3a, 1, b'x'], true),
            Err(ProbeError::SessionProtocol(
                SessionError::UnknownUpdateConversation
            ))
        ));
    }
}
