//! Independent projection of encrypted first-party push updates.
use crate::{ProbeError, session::SessionError};
use handover_core::messaging::{Conversation, Message as CoreMessage};
use prost::Message;
use zeroize::{Zeroize, Zeroizing};

/// A bounded observer result. It does not attest account connectivity.
pub enum Update {
    Conversations(Vec<Conversation>),
    Messages(Vec<CoreMessage>),
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
                for record in crate::history::decode_records(conversation, vec![bytes.to_vec()])? {
                    if let Some(previous) = messages.iter().find(|item| item.id == record.id) {
                        if previous != &record {
                            return Err(invalid());
                        }
                    } else {
                        messages.push(record);
                    }
                }
            }
            Ok(Update::Messages(messages))
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
}
impl Drop for Thread {
    fn drop(&mut self) {
        self.id.zeroize();
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
