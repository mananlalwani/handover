//! Minimal first-party conversation projection into backend-independent models.
use crate::{ProbeError, session::SessionError};
use handover_core::messaging::{
    Conversation, ConversationId, ConversationKind, MessagingAccountId, Participant, TransportKind,
    validate_conversation,
};
use prost::Message;
use std::collections::BTreeSet;
use zeroize::{Zeroize, Zeroizing};

pub(crate) fn decode(account: &str, bytes: &[u8]) -> Result<Conversation, ProbeError> {
    let mut wire = WireConversation::decode(bytes)
        .map_err(|_| ProbeError::SessionProtocol(SessionError::ConversationEncoding))?;
    // A phone may repeat a local SIM identity without its self flag. Link
    // aliases only through an identity or exact routing address that this same
    // conversation explicitly marks as self.
    let own_addresses: Vec<Zeroizing<String>> = wire
        .participants
        .iter()
        .filter(|participant| participant.is_self)
        .filter_map(|participant| participant.identity.as_ref())
        .filter(|identity| !identity.id.is_empty())
        .map(|identity| Zeroizing::new(identity.id.clone()))
        .collect();
    let own_keys: Vec<Zeroizing<String>> = wire
        .participants
        .iter()
        .filter(|participant| participant.is_self)
        .filter_map(|participant| participant.identity.as_ref())
        .filter(|identity| !identity.participant_id.is_empty())
        .map(|identity| Zeroizing::new(identity.participant_id.clone()))
        .collect();
    let participants = wire
        .participants
        .iter_mut()
        .map(|participant| {
            let identity = participant
                .identity
                .as_mut()
                .ok_or(ProbeError::SessionProtocol(
                    SessionError::ParticipantIdentity,
                ))?;
            participant.is_self |= own_addresses
                .iter()
                .any(|address| address.as_str() == identity.id)
                || own_keys
                    .iter()
                    .any(|key| key.as_str() == identity.participant_id);
            if identity.id.is_empty() && identity.participant_id.is_empty() {
                return Err(ProbeError::SessionProtocol(
                    SessionError::ParticipantIdentity,
                ));
            }
            let address = Some(std::mem::take(&mut identity.id));
            let id = if participant.is_self {
                format!("self:{account}")
            } else if identity.participant_id.is_empty() {
                format!(
                    "peer:{}:{}",
                    identity.kind,
                    address.as_deref().unwrap_or_default()
                )
            } else {
                format!("peer:{}", identity.participant_id)
            };
            let name = if !participant.display_name.is_empty() {
                std::mem::take(&mut participant.display_name)
            } else {
                std::mem::take(&mut participant.full_name)
            };
            Ok(Participant {
                local_id: id,
                // A self identity can cover multiple SIM aliases. Expose its
                // attested role without choosing one alias as the account name
                // or number. Routing still uses the conversation on the phone.
                display_name: (!participant.is_self && !name.is_empty()).then_some(name),
                address: if participant.is_self {
                    None
                } else {
                    address.filter(|value| !value.is_empty())
                },
                is_self: participant.is_self,
            })
        })
        .collect::<Result<Vec<_>, ProbeError>>()?;
    let mut unique: Vec<Participant> = Vec::with_capacity(participants.len());
    for participant in participants {
        if let Some(existing) = unique
            .iter_mut()
            .find(|existing| existing.local_id == participant.local_id)
        {
            if existing.is_self != participant.is_self {
                return Err(ProbeError::SessionProtocol(
                    SessionError::ParticipantRoleConflict,
                ));
            }
            if existing.address != participant.address {
                return Err(ProbeError::SessionProtocol(
                    SessionError::ParticipantAddressConflict,
                ));
            }
            match (&existing.display_name, &participant.display_name) {
                (Some(first), Some(second)) if first != second => {
                    return Err(ProbeError::SessionProtocol(
                        SessionError::ParticipantNameConflict,
                    ));
                }
                (None, Some(_)) => existing.display_name = participant.display_name,
                _ => {}
            }
        } else {
            unique.push(participant);
        }
    }
    let participants = unique;
    let title = std::mem::take(&mut wire.title);
    let conversation = Conversation {
        id: ConversationId::new(
            MessagingAccountId::new(account),
            std::mem::take(&mut wire.id),
        ),
        // Older multi-party threads may omit the group flag. Distinct
        // normalized members still attest a group after self aliases merge.
        kind: if wire.group || participants.len() > 2 {
            ConversationKind::Group
        } else {
            ConversationKind::Direct
        },
        // Network family and send/history capabilities need their own evidence.
        transport: TransportKind::Unknown,
        title: (!title.is_empty()).then_some(title),
        participants,
        latest_message_id: None,
        last_activity_at: wire.timestamp_micros.filter(|value| *value > 0),
        unread_count: wire
            .unread_count
            .map(u64::try_from)
            .transpose()
            .map_err(|_| ProbeError::SessionProtocol(SessionError::InvalidUnreadCount))?,
        cursor: None,
        capabilities: BTreeSet::new(),
    };
    validate_conversation(&conversation).map_err(|error| {
        use handover_core::messaging::ValidationError;
        let category = match error {
            ValidationError::InvalidId(_) => SessionError::ConversationIdentifier,
            ValidationError::InvalidLabel(_) => SessionError::ConversationText,
            ValidationError::TooManyParticipants(_) => SessionError::TooManyParticipants,
            ValidationError::EmptyParticipants => SessionError::MissingParticipants,
            ValidationError::DuplicateParticipant(_) => SessionError::DuplicateParticipants,
            ValidationError::DirectWithManyParticipants => SessionError::DirectParticipants,
            _ => SessionError::ConversationModel,
        };
        ProbeError::SessionProtocol(category)
    })?;
    Ok(conversation)
}

// SI/Wv and Qv/Mv in the first-party script. Message previews, media, and
// provider-specific flags are skipped rather than exposed or guessed.
#[derive(Message)]
#[prost(skip_debug)]
struct WireConversation {
    #[prost(string, tag = "1")]
    id: String,
    #[prost(string, tag = "2")]
    title: String,
    #[prost(int64, optional, tag = "5")]
    timestamp_micros: Option<i64>,
    #[prost(int32, optional, tag = "6")]
    unread_count: Option<i32>,
    #[prost(bool, tag = "10")]
    group: bool,
    #[prost(message, repeated, tag = "20")]
    participants: Vec<WireParticipant>,
}
impl Drop for WireConversation {
    fn drop(&mut self) {
        self.id.zeroize();
        self.title.zeroize();
    }
}
#[derive(Message)]
#[prost(skip_debug)]
struct WireParticipant {
    #[prost(message, optional, tag = "1")]
    identity: Option<WireIdentity>,
    #[prost(string, tag = "3")]
    full_name: String,
    #[prost(bool, tag = "6")]
    is_self: bool,
    #[prost(string, tag = "15")]
    display_name: String,
}
impl Drop for WireParticipant {
    fn drop(&mut self) {
        self.full_name.zeroize();
        self.display_name.zeroize();
    }
}
#[derive(Message)]
#[prost(skip_debug)]
struct WireIdentity {
    #[prost(int32, tag = "1")]
    kind: i32,
    #[prost(string, tag = "2")]
    id: String,
    #[prost(string, tag = "3")]
    participant_id: String,
}
impl Drop for WireIdentity {
    fn drop(&mut self) {
        self.id.zeroize();
        self.participant_id.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wire() -> WireConversation {
        WireConversation {
            id: "thread".into(),
            title: "Fixture".into(),
            timestamp_micros: Some(1234000),
            unread_count: None,
            group: false,
            participants: vec![WireParticipant {
                identity: Some(WireIdentity {
                    kind: 1,
                    id: "+15555550100".into(),
                    participant_id: String::new(),
                }),
                full_name: "Fixture person".into(),
                display_name: String::new(),
                is_self: false,
            }],
        }
    }
    #[test]
    fn an_attested_self_address_links_unflagged_aliases_to_the_same_self() {
        let mut conversation = wire();
        for (key, own) in [("sim-one", true), ("sim-alias", false)] {
            conversation.participants.push(WireParticipant {
                identity: Some(WireIdentity {
                    kind: 1,
                    id: "+15555550999".into(),
                    participant_id: key.into(),
                }),
                full_name: "Own alias".into(),
                display_name: String::new(),
                is_self: own,
            });
        }
        let decoded = decode("gmessages-fixture", &conversation.encode_to_vec()).unwrap();
        assert_eq!(decoded.participants.len(), 2);
        assert_eq!(
            decoded
                .participants
                .iter()
                .filter(|participant| participant.is_self)
                .count(),
            1
        );
    }

    #[test]
    fn multiple_self_sim_keys_form_one_self_participant_in_a_direct_thread() {
        let mut conversation = wire();
        for key in ["sim-one", "sim-two"] {
            conversation.participants.push(WireParticipant {
                identity: Some(WireIdentity {
                    kind: 1,
                    id: "+15555550999".into(),
                    participant_id: key.into(),
                }),
                full_name: "Self fixture".into(),
                display_name: String::new(),
                is_self: true,
            });
        }
        let decoded = decode("gmessages-fixture", &conversation.encode_to_vec()).unwrap();
        assert_eq!(decoded.participants.len(), 2);
        assert_eq!(
            decoded
                .participants
                .iter()
                .filter(|participant| participant.is_self)
                .count(),
            1
        );
    }

    #[test]
    fn self_aliases_share_one_provider_identity_without_picking_a_name_or_number() {
        let mut conversation = wire();
        let first = &mut conversation.participants[0];
        first.is_self = true;
        first.identity.as_mut().unwrap().participant_id = "self-key".into();
        conversation.participants.push(WireParticipant {
            identity: Some(WireIdentity {
                kind: 1,
                id: "+15555550200".into(),
                participant_id: "self-key".into(),
            }),
            full_name: "Other SIM name".into(),
            display_name: String::new(),
            is_self: true,
        });
        let decoded = decode("gmessages-fixture", &conversation.encode_to_vec()).unwrap();
        assert_eq!(decoded.participants.len(), 1);
        assert!(decoded.participants[0].is_self);
        assert!(decoded.participants[0].display_name.is_none());
        assert!(decoded.participants[0].address.is_none());
    }

    #[test]
    fn provider_participant_keys_distinguish_contacts_sharing_an_address() {
        let mut conversation = wire();
        conversation.participants[0]
            .identity
            .as_mut()
            .unwrap()
            .participant_id = "first-person".into();
        conversation.participants.push(WireParticipant {
            identity: Some(WireIdentity {
                kind: 1,
                id: "+15555550100".into(),
                participant_id: "second-person".into(),
            }),
            full_name: "Second fixture".into(),
            display_name: String::new(),
            is_self: false,
        });
        let decoded = decode("gmessages-fixture", &conversation.encode_to_vec()).unwrap();
        assert_eq!(decoded.participants[0].local_id, "peer:first-person");
        assert_eq!(decoded.participants[1].local_id, "peer:second-person");
        assert_eq!(
            decoded.participants[0].address,
            decoded.participants[1].address
        );
    }

    #[test]
    fn distinct_members_attest_a_group_when_the_group_flag_is_absent() {
        let mut conversation = wire();
        for key in ["second", "third"] {
            conversation.participants.push(WireParticipant {
                identity: Some(WireIdentity {
                    kind: 1,
                    id: String::new(),
                    participant_id: key.into(),
                }),
                full_name: String::new(),
                display_name: String::new(),
                is_self: false,
            });
        }
        let decoded = decode("gmessages-fixture", &conversation.encode_to_vec()).unwrap();
        assert_eq!(decoded.kind, ConversationKind::Group);
        assert_eq!(decoded.participants.len(), 3);
    }

    #[test]
    fn self_only_thread_collapses_an_unflagged_routing_alias() {
        let mut conversation = wire();
        conversation.participants.push(WireParticipant {
            identity: Some(WireIdentity {
                kind: 1,
                id: "+15555550100".into(),
                participant_id: String::new(),
            }),
            full_name: "Self fixture".into(),
            display_name: String::new(),
            is_self: true,
        });
        let decoded = decode("gmessages-fixture", &conversation.encode_to_vec()).unwrap();
        assert_eq!(decoded.participants.len(), 1);
        assert!(decoded.participants[0].is_self);
        assert!(decoded.participants[0].address.is_none());
        assert!(decoded.participants[0].display_name.is_none());
    }

    #[test]
    fn identical_provider_participants_collapse_but_conflicts_are_rejected() {
        let mut duplicate = wire();
        duplicate.participants.push(WireParticipant {
            identity: Some(WireIdentity {
                kind: 1,
                id: "+15555550100".into(),
                participant_id: String::new(),
            }),
            full_name: "Fixture person".into(),
            display_name: String::new(),
            is_self: false,
        });
        assert_eq!(
            decode("gmessages-fixture", &duplicate.encode_to_vec())
                .unwrap()
                .participants
                .len(),
            1
        );
        duplicate.participants[1].full_name.clear();
        assert_eq!(
            decode("gmessages-fixture", &duplicate.encode_to_vec())
                .unwrap()
                .participants[0]
                .display_name
                .as_deref(),
            Some("Fixture person")
        );
        duplicate.participants[1].full_name = "Conflicting name".into();
        assert!(decode("gmessages-fixture", &duplicate.encode_to_vec()).is_err());
    }

    #[test]
    fn projects_attested_fields_without_inventing_transport_or_capabilities() {
        let result = decode("gmessages-fixture", &wire().encode_to_vec()).unwrap();
        assert_eq!(result.id.local_id, "thread");
        assert_eq!(result.last_activity_at, Some(1234000));
        assert_eq!(result.unread_count, None);
        assert_eq!(
            result.participants[0].address.as_deref(),
            Some("+15555550100")
        );
        assert_eq!(result.transport, TransportKind::Unknown);
        assert!(result.capabilities.is_empty());
        assert!(result.latest_message_id.is_none());
        let mut bad = wire();
        bad.unread_count = Some(-1);
        assert!(decode("gmessages-fixture", &bad.encode_to_vec()).is_err());
        let mut bad = wire();
        bad.participants.clear();
        assert!(decode("gmessages-fixture", &bad.encode_to_vec()).is_err());
        let mut bad = wire();
        bad.participants.push(WireParticipant {
            identity: Some(WireIdentity {
                kind: 1,
                id: "+15555550100".into(),
                participant_id: String::new(),
            }),
            full_name: "Conflicting name".into(),
            display_name: String::new(),
            is_self: false,
        });
        assert!(decode("gmessages-fixture", &bad.encode_to_vec()).is_err());
    }
}
