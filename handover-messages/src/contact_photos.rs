//! Match address-book photos by exact normalized addresses, never display names.
use handover_core::{Contact, Conversation, ConversationKind, Participant};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Default)]
pub struct ContactPhotos(BTreeMap<String, Option<Arc<str>>>);

impl ContactPhotos {
    pub fn replace(&mut self, contacts: Vec<Contact>) {
        self.0.clear();
        for contact in contacts {
            let photo = contact
                .photo
                .filter(|photo| !photo.is_empty() && photo.len() <= 48 * 1024)
                .map(Arc::<str>::from);
            for address in contact.phones.iter().chain(&contact.emails) {
                let Some(address) = key(address) else {
                    continue;
                };
                self.0
                    .entry(address)
                    .and_modify(|existing| {
                        if *existing != photo {
                            *existing = None;
                        }
                    })
                    .or_insert_with(|| photo.clone());
            }
        }
    }

    pub fn participant(&self, participant: &Participant) -> Option<Arc<str>> {
        self.0
            .get(&key(participant.address.as_deref()?)?)
            .and_then(Clone::clone)
    }

    pub fn conversation(&self, conversation: &Conversation) -> Option<Arc<str>> {
        if conversation.kind != ConversationKind::Direct {
            return None;
        }
        let mut photos = conversation
            .participants
            .iter()
            .filter(|p| !p.is_self)
            .filter_map(|p| self.participant(p));
        let first = photos.next()?;
        if photos.any(|photo| photo != first) {
            None
        } else {
            Some(first)
        }
    }
}

fn key(address: &str) -> Option<String> {
    let address = address.trim();
    if let Some((local, domain)) = address.split_once('@') {
        if local.is_empty() || domain.is_empty() || domain.contains('@') {
            return None;
        }
        return Some(format!("email:{local}@{}", domain.to_ascii_lowercase()));
    }
    if address
        .chars()
        .any(|c| !c.is_ascii_digit() && !"+ -().".contains(c))
    {
        return None;
    }
    let digits: String = address.chars().filter(char::is_ascii_digit).collect();
    (7..=15)
        .contains(&digits.len())
        .then(|| format!("phone:{digits}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use handover_core::DeviceId;
    fn contact(phone: &str, photo: &str) -> Contact {
        Contact {
            device_id: DeviceId::new("phone"),
            local_id: photo.into(),
            display_name: "Same name".into(),
            phones: vec![phone.into()],
            emails: vec![],
            photo: Some(photo.into()),
        }
    }
    fn participant(phone: &str) -> Participant {
        Participant {
            local_id: "peer".into(),
            display_name: Some("Same name".into()),
            address: Some(phone.into()),
            is_self: false,
        }
    }
    #[test]
    fn exact_addresses_match_formatting_without_guessing_country_or_names() {
        let mut photos = ContactPhotos::default();
        photos.replace(vec![contact("+1 (312) 555-0100", "photo")]);
        assert_eq!(
            photos.participant(&participant("+13125550100")).as_deref(),
            Some("photo")
        );
        assert!(photos.participant(&participant("3125550100")).is_none());
        assert!(photos.participant(&participant("Same name")).is_none());
        photos.replace(vec![
            contact("+13125550100", "one"),
            contact("+1 312 555 0100", "two"),
        ]);
        assert!(photos.participant(&participant("+13125550100")).is_none());
        photos.replace(vec![]);
        assert!(photos.participant(&participant("+13125550100")).is_none());
    }
}
