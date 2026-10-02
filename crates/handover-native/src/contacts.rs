//! Assemble bounded native contact generations before replacing normalized state.
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use handover_core::ContactsSyncFailure;

use crate::protocol::WireContact;

const MAX_CONTACTS: usize = 4096;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_CHUNKS: u32 = 256;
const MAX_RECORD_BYTES: usize = 56 * 1024;
const GENERATION_TIMEOUT: Duration = Duration::from_secs(30);

struct Active {
    generation: String,
    next_index: u32,
    started: Instant,
    bytes: usize,
    ids: HashSet<String>,
    contacts: Vec<WireContact>,
}

#[derive(Default)]
pub(crate) struct ContactChunks {
    active: Option<Active>,
    closed: VecDeque<String>,
}

fn valid_contact(contact: &WireContact) -> bool {
    !contact.local_id.is_empty()
        && contact.local_id.len() <= 256
        && !contact.local_id.chars().any(char::is_control)
        && contact.display_name.chars().count() <= 256
        && contact.phones.len() <= 16
        && contact.emails.len() <= 16
        && contact
            .phones
            .iter()
            .chain(&contact.emails)
            .all(|value| value.chars().count() <= 256)
        && contact
            .photo
            .as_ref()
            .is_none_or(|photo| photo.len() <= 48 * 1024)
}

impl ContactChunks {
    fn close(&mut self, generation: String) {
        if !self.closed.contains(&generation) {
            self.closed.push_back(generation);
            if self.closed.len() > 64 {
                self.closed.pop_front();
            }
        }
    }

    pub(crate) fn abort(&mut self, generation: &str) {
        if generation.is_empty() || generation.len() > 64 {
            return;
        }
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.generation == generation)
        {
            self.active = None;
        }
        self.close(generation.to_string());
    }

    pub(crate) fn expire(&mut self, now: Instant) -> bool {
        if self
            .active
            .as_ref()
            .is_some_and(|active| now.duration_since(active.started) >= GENERATION_TIMEOUT)
        {
            let active = self.active.take().expect("active generation");
            self.close(active.generation);
            return true;
        }
        false
    }

    pub(crate) fn interrupt(&mut self) -> bool {
        if let Some(active) = self.active.take() {
            self.close(active.generation);
            true
        } else {
            false
        }
    }

    pub(crate) fn push(
        &mut self,
        generation: String,
        index: u32,
        done: bool,
        contacts: Vec<WireContact>,
        now: Instant,
    ) -> Result<Option<Vec<WireContact>>, ContactsSyncFailure> {
        if generation.is_empty()
            || generation.len() > 64
            || !generation
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || self.closed.contains(&generation)
        {
            return Err(ContactsSyncFailure::Rejected);
        }
        if self.expire(now) && index != 0 {
            return Err(ContactsSyncFailure::Interrupted);
        }
        // Expiry may have closed this generation during this push.
        if self.closed.contains(&generation) {
            return Err(ContactsSyncFailure::Rejected);
        }
        if index == 0 {
            if let Some(active) = self.active.take() {
                self.close(active.generation.clone());
                if active.generation == generation {
                    return Err(ContactsSyncFailure::Rejected);
                }
            }
            self.active = Some(Active {
                generation: generation.clone(),
                next_index: 0,
                started: now,
                bytes: 0,
                ids: HashSet::new(),
                contacts: Vec::new(),
            });
        }
        let result = self.append(&generation, index, done, contacts);
        if result.is_err() {
            self.abort(&generation);
        }
        result
    }

    fn append(
        &mut self,
        generation: &str,
        index: u32,
        done: bool,
        contacts: Vec<WireContact>,
    ) -> Result<Option<Vec<WireContact>>, ContactsSyncFailure> {
        let active = self.active.as_mut().ok_or(ContactsSyncFailure::Rejected)?;
        if active.generation != generation
            || active.next_index != index
            || index >= MAX_CHUNKS
            || (!done && contacts.is_empty())
        {
            return Err(ContactsSyncFailure::Rejected);
        }
        if active.contacts.len().saturating_add(contacts.len()) > MAX_CONTACTS {
            return Err(ContactsSyncFailure::TooLarge);
        }
        for contact in contacts {
            if !valid_contact(&contact) || !active.ids.insert(contact.local_id.clone()) {
                return Err(ContactsSyncFailure::Rejected);
            }
            let bytes = serde_json::to_vec(&contact)
                .map_err(|_| ContactsSyncFailure::Rejected)?
                .len()
                + 1;
            active.bytes = active.bytes.saturating_add(bytes);
            if bytes > MAX_RECORD_BYTES || active.bytes > MAX_BYTES {
                return Err(ContactsSyncFailure::TooLarge);
            }
            active.contacts.push(contact);
        }
        active.next_index += 1;
        if done {
            let active = self.active.take().expect("completed generation");
            self.close(active.generation);
            Ok(Some(active.contacts))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn contact(id: &str) -> WireContact {
        WireContact {
            local_id: id.into(),
            display_name: "Contact".into(),
            phones: vec![],
            emails: vec![],
            photo: None,
        }
    }
    #[test]
    fn legacy_requests_and_snapshots_remain_compatible() {
        let request: crate::protocol::Message =
            serde_json::from_str(r#"{"type":"contacts_request","protocol":1}"#).unwrap();
        assert!(matches!(
            request,
            crate::protocol::Message::ContactsRequest { chunked: false, .. }
        ));
        let snapshot: crate::protocol::Message =
            serde_json::from_str(r#"{"type":"contacts_sync","protocol":1,"contacts":[]}"#).unwrap();
        assert!(matches!(
            snapshot,
            crate::protocol::Message::ContactsSync { complete: true, .. }
        ));
        let snapshot: crate::protocol::Message = serde_json::from_str(
            r#"{"type":"contacts_sync","protocol":1,"contacts":[],"complete":false}"#,
        )
        .unwrap();
        assert!(matches!(
            snapshot,
            crate::protocol::Message::ContactsSync {
                complete: false,
                ..
            }
        ));
    }
    #[test]
    fn only_complete_ordered_generations_are_published() {
        let mut chunks = ContactChunks::default();
        let now = Instant::now();
        assert_eq!(
            chunks.push("generation".into(), 0, false, vec![contact("a")], now),
            Ok(None)
        );
        assert_eq!(
            chunks.push("generation".into(), 1, true, vec![contact("b")], now),
            Ok(Some(vec![contact("a"), contact("b")]))
        );
        assert_eq!(
            chunks.push("generation".into(), 0, true, vec![contact("a")], now),
            Err(ContactsSyncFailure::Rejected)
        );
    }
    #[test]
    fn gaps_duplicates_abort_and_late_chunks_cannot_commit() {
        let now = Instant::now();
        for bad_index in [0, 2] {
            let mut chunks = ContactChunks::default();
            chunks
                .push("generation".into(), 0, false, vec![contact("a")], now)
                .unwrap();
            assert_eq!(
                chunks.push(
                    "generation".into(),
                    bad_index,
                    true,
                    vec![contact("b")],
                    now
                ),
                Err(ContactsSyncFailure::Rejected)
            );
            assert_eq!(
                chunks.push("generation".into(), 1, true, vec![contact("b")], now),
                Err(ContactsSyncFailure::Rejected)
            );
        }
        let mut chunks = ContactChunks::default();
        chunks
            .push("generation".into(), 0, false, vec![contact("a")], now)
            .unwrap();
        assert_eq!(
            chunks.push("generation".into(), 1, true, vec![contact("a")], now),
            Err(ContactsSyncFailure::Rejected)
        );
    }
    #[test]
    fn interrupted_generation_expires_and_next_generation_can_finish() {
        let mut chunks = ContactChunks::default();
        let now = Instant::now();
        chunks
            .push("old".into(), 0, false, vec![contact("a")], now)
            .unwrap();
        assert!(chunks.expire(now + GENERATION_TIMEOUT));
        assert_eq!(
            chunks.push(
                "old".into(),
                1,
                true,
                vec![contact("b")],
                now + GENERATION_TIMEOUT
            ),
            Err(ContactsSyncFailure::Rejected)
        );
        assert_eq!(
            chunks.push(
                "new".into(),
                0,
                true,
                vec![contact("c")],
                now + GENERATION_TIMEOUT
            ),
            Ok(Some(vec![contact("c")]))
        );
    }
    #[test]
    fn expired_generation_cannot_restart_with_a_replayed_first_chunk() {
        let mut chunks = ContactChunks::default();
        let now = Instant::now();
        chunks
            .push("old".into(), 0, false, vec![contact("a")], now)
            .unwrap();
        assert_eq!(
            chunks.push(
                "old".into(),
                0,
                true,
                vec![contact("a")],
                now + GENERATION_TIMEOUT
            ),
            Err(ContactsSyncFailure::Rejected)
        );
        assert_eq!(
            chunks.push(
                "new".into(),
                0,
                true,
                vec![contact("b")],
                now + GENERATION_TIMEOUT
            ),
            Ok(Some(vec![contact("b")]))
        );
    }

    #[test]
    fn counts_and_bytes_are_bounded_before_publication() {
        let now = Instant::now();
        let mut chunks = ContactChunks::default();
        assert_eq!(
            chunks.push(
                "too-many".into(),
                0,
                true,
                (0..=MAX_CONTACTS)
                    .map(|index| contact(&index.to_string()))
                    .collect(),
                now
            ),
            Err(ContactsSyncFailure::TooLarge)
        );
        let mut chunks = ContactChunks::default();
        for index in 0..MAX_CHUNKS {
            let mut item = contact(&index.to_string());
            item.photo = Some("a".repeat(48 * 1024));
            let result = chunks.push("bytes".into(), index, false, vec![item], now);
            if result == Err(ContactsSyncFailure::TooLarge) {
                return;
            }
            assert_eq!(result, Ok(None));
        }
        panic!("byte limit must abort the generation");
    }
}
