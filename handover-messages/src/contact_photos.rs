//! Resolve address-book names and photos by normalized addresses.
use handover_core::{Contact, Conversation, ConversationKind, Participant};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Default, PartialEq)]
pub struct ContactPhotos {
    photos: BTreeMap<String, Option<Arc<str>>>,
    names: BTreeMap<String, Option<String>>,
}

impl ContactPhotos {
    pub fn replace(&mut self, contacts: Vec<Contact>) -> bool {
        let mut next = Self::default();
        for contact in contacts {
            if !contact.display_name.trim().is_empty() {
                for address in contact
                    .phones
                    .iter()
                    .chain(&contact.emails)
                    .filter_map(|address| key(address))
                {
                    next.names
                        .entry(address)
                        .and_modify(|name| {
                            if name.as_deref() != Some(contact.display_name.as_str()) {
                                *name = None;
                            }
                        })
                        .or_insert_with(|| Some(contact.display_name.clone()));
                }
            }
            let Some(photo) = contact
                .photo
                .filter(|photo| !photo.is_empty() && photo.len() <= 48 * 1024)
                .map(Arc::<str>::from)
            else {
                continue;
            };
            for address in contact.phones.iter().chain(&contact.emails) {
                let Some(address) = key(address) else {
                    continue;
                };
                next.photos
                    .entry(address)
                    .and_modify(|existing| {
                        if existing.as_ref() != Some(&photo) {
                            *existing = None;
                        }
                    })
                    .or_insert_with(|| Some(photo.clone()));
            }
        }
        if *self == next {
            return false;
        }
        *self = next;
        true
    }

    pub fn label(&self, participant: &Participant) -> String {
        participant
            .address
            .as_deref()
            .and_then(key)
            .and_then(|address| self.names.get(&address))
            .and_then(Option::as_deref)
            .or(participant
                .display_name
                .as_deref()
                .filter(|name| !name.trim().is_empty()))
            .or(participant.address.as_deref())
            .unwrap_or("Contact")
            .to_owned()
    }

    pub fn participant(&self, participant: &Participant) -> Option<Arc<str>> {
        self.photos
            .get(&key(participant.address.as_deref()?)?)
            .and_then(Clone::clone)
    }

    pub fn conversation_photos(&self, conversation: &Conversation) -> Vec<Arc<str>> {
        let mut photos = Vec::new();
        for photo in conversation
            .participants
            .iter()
            .filter(|p| !p.is_self)
            .filter_map(|p| self.participant(p))
        {
            if !photos.contains(&photo) {
                photos.push(photo);
            }
            if photos.len() == 4 {
                break;
            }
        }
        if conversation.kind == ConversationKind::Direct && photos.len() > 1 {
            Vec::new()
        } else {
            photos
        }
    }
}

pub(crate) fn key(address: &str) -> Option<String> {
    let address = address.trim();
    if let Some((local, domain)) = address.split_once('@') {
        if local.is_empty() || domain.is_empty() || domain.contains('@') {
            return None;
        }
        return Some(format!(
            "email:{}@{}",
            local.to_ascii_lowercase(),
            domain.to_ascii_lowercase()
        ));
    }
    if address
        .chars()
        .any(|c| !c.is_ascii_digit() && !"+ -().".contains(c))
    {
        return None;
    }
    let digits: String = address.chars().filter(char::is_ascii_digit).collect();
    if !(7..=15).contains(&digits.len()) {
        return None;
    }
    // Preserve the QML client's explicit NANP equivalence. Do not strip
    // arbitrary international prefixes or match numbers by their suffix.
    let national = if digits.len() == 11 && digits.starts_with('1') {
        &digits[1..]
    } else {
        &digits
    };
    if national.len() == 10
        && matches!(national.as_bytes()[0], b'2'..=b'9')
        && matches!(national.as_bytes()[3], b'2'..=b'9')
    {
        Some(format!("nanp:{national}"))
    } else {
        Some(format!("phone:{digits}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use handover_core::{ConversationId, DeviceId, MessagingAccountId, TransportKind};
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

    fn conversation(kind: ConversationKind, participants: Vec<Participant>) -> Conversation {
        Conversation {
            id: ConversationId::new(MessagingAccountId::new("account"), "conversation"),
            kind,
            transport: TransportKind::Sms,
            title: None,
            participants,
            latest_message_id: None,
            last_activity_at: None,
            unread_count: None,
            cursor: None,
            capabilities: Default::default(),
        }
    }

    fn contact_with_email(email: &str, photo: &str) -> Contact {
        Contact {
            emails: vec![email.into()],
            ..contact("", photo)
        }
    }

    fn participant_with_address(address: &str, is_self: bool) -> Participant {
        Participant {
            local_id: address.into(),
            display_name: None,
            address: Some(address.into()),
            is_self,
        }
    }
    #[test]
    fn matching_accepts_formatting_and_nanp_country_code_without_using_names() {
        let mut photos = ContactPhotos::default();
        photos.replace(vec![contact("+1 (312) 555-0100", "photo")]);
        assert_eq!(
            photos.participant(&participant("+13125550100")).as_deref(),
            Some("photo")
        );
        assert_eq!(
            photos.participant(&participant("3125550100")).as_deref(),
            Some("photo")
        );
        assert!(photos.participant(&participant("Same name")).is_none());
        photos.replace(vec![
            contact("+13125550100", "one"),
            contact("+1 312 555 0100", "two"),
        ]);
        assert!(photos.participant(&participant("+13125550100")).is_none());
        photos.replace(vec![]);
        assert!(photos.participant(&participant("+13125550100")).is_none());
    }

    #[test]
    fn invalid_nanp_and_non_nanp_numbers_do_not_suffix_match() {
        let mut photos = ContactPhotos::default();
        photos.replace(vec![
            contact("+1 123 555 0100", "invalid-nanp"),
            contact("+44 20 7946 0018", "uk-number"),
            contact("+1 312 555 0911", "ends-in-short-code"),
        ]);

        assert!(photos.participant(&participant("1235550100")).is_none());
        assert!(photos.participant(&participant("020 7946 0018")).is_none());
        assert!(photos.participant(&participant("911")).is_none());
        assert_eq!(
            photos
                .participant(&participant("+44 (20) 7946-0018"))
                .as_deref(),
            Some("uk-number")
        );
    }

    #[test]
    fn email_matching_ignores_ascii_case_in_local_part_and_domain() {
        let mut photos = ContactPhotos::default();
        photos.replace(vec![contact_with_email("Alex.Example@Mail.test", "photo")]);

        assert_eq!(
            photos
                .participant(&participant("alex.example@mail.TEST"))
                .as_deref(),
            Some("photo")
        );
    }

    #[test]
    fn contact_without_photo_after_photo_does_not_hide_the_unique_photo() {
        let with_photo = contact("+13125550100", "photo");
        let without_photo = Contact {
            photo: None,
            ..contact("+1 (312) 555-0100", "unused")
        };
        let mut photos = ContactPhotos::default();

        photos.replace(vec![with_photo.clone(), without_photo.clone()]);
        assert_eq!(
            photos.participant(&participant("+13125550100")).as_deref(),
            Some("photo")
        );
    }

    #[test]
    fn contact_without_photo_before_photo_does_not_hide_the_unique_photo() {
        let with_photo = contact("+13125550100", "photo");
        let without_photo = Contact {
            photo: None,
            ..contact("+1 (312) 555-0100", "unused")
        };
        let mut photos = ContactPhotos::default();
        photos.replace(vec![without_photo, with_photo]);
        assert_eq!(
            photos.participant(&participant("+13125550100")).as_deref(),
            Some("photo")
        );
    }

    #[test]
    fn photo_conflict_stays_ambiguous_after_a_photo_less_duplicate() {
        let without_photo = Contact {
            photo: None,
            ..contact("+13125550100", "unused")
        };
        let mut photos = ContactPhotos::default();
        photos.replace(vec![
            contact("+13125550100", "one"),
            contact("+1 (312) 555-0100", "two"),
            without_photo,
        ]);

        assert!(photos.participant(&participant("+13125550100")).is_none());
    }

    #[test]
    fn names_do_not_require_photos_and_unchanged_refreshes_are_stable() {
        let mut contacts = ContactPhotos::default();
        let alex = Contact {
            display_name: "Alex Example".into(),
            photo: None,
            ..contact("+13125550100", "unused")
        };
        assert!(contacts.replace(vec![alex.clone()]));
        assert_eq!(contacts.label(&participant("3125550100")), "Alex Example");
        assert!(!contacts.replace(vec![alex]));
        assert!(contacts.replace(vec![]));
        assert_eq!(contacts.label(&participant("3125550100")), "Same name");
    }

    #[test]
    fn conflicting_contact_names_fall_back_to_reported_identity() {
        let mut contacts = ContactPhotos::default();
        contacts.replace(vec![
            Contact {
                display_name: "Alex Example".into(),
                ..contact("+13125550100", "one")
            },
            Contact {
                display_name: "Another Person".into(),
                ..contact("3125550100", "two")
            },
        ]);
        assert_eq!(contacts.label(&participant("3125550100")), "Same name");
    }

    #[test]
    fn group_photos_are_distinct_ordered_bounded_and_exclude_self() {
        let mut photos = ContactPhotos::default();
        photos.replace(vec![
            contact("+13125550101", "one"),
            contact("+13125550102", "two"),
            contact("+13125550103", "three"),
            contact("+13125550104", "four"),
            contact("+13125550105", "five"),
        ]);
        let group = conversation(
            ConversationKind::Group,
            vec![
                participant_with_address("+13125550105", true),
                participant_with_address("+13125550102", false),
                participant_with_address("+13125550101", false),
                participant_with_address("+13125550102", false),
                participant_with_address("+13125550103", false),
                participant_with_address("+13125550104", false),
                participant_with_address("+13125550105", false),
            ],
        );

        let group_photos = photos.conversation_photos(&group);
        let result: Vec<_> = group_photos.iter().map(|photo| photo.as_ref()).collect();
        assert_eq!(result, vec!["two", "one", "three", "four"]);
    }
}
