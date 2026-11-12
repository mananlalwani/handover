//! Private aliases from authenticated conversation records. Never exposed over IPC.
use handover_core::{Conversation, ConversationId, Message};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

#[derive(Default)]
pub(crate) struct ActorIdentities {
    entries: BTreeMap<ConversationId, Zeroizing<Vec<String>>>,
    bytes: usize,
}
impl ActorIdentities {
    pub(crate) fn remember(&mut self, id: ConversationId, actors: Zeroizing<Vec<String>>) {
        if let Some(old) = self.entries.remove(&id) {
            self.bytes -= old.iter().map(String::len).sum::<usize>();
        }
        if actors.is_empty() {
            return;
        }
        let bytes = actors.iter().map(String::len).sum::<usize>();
        if bytes > 4 * 1024 * 1024 {
            return;
        }
        while self.entries.len() >= 10_000 || self.bytes + bytes > 4 * 1024 * 1024 {
            let Some((_, old)) = self.entries.pop_first() else {
                break;
            };
            self.bytes -= old.iter().map(String::len).sum::<usize>();
        }
        self.bytes += bytes;
        self.entries.insert(id, actors);
    }
    pub(crate) fn normalize(&self, conversation: &Conversation, messages: &mut [Message]) {
        let Some(aliases) = self.entries.get(&conversation.id) else {
            return;
        };
        let Some(own) = conversation
            .participants
            .iter()
            .find(|participant| participant.is_self)
        else {
            return;
        };
        for message in messages
            .iter_mut()
            .filter(|message| message.id.conversation_id == conversation.id)
        {
            for reaction in &mut message.reactions {
                for actor in &mut reaction.participant_ids {
                    if aliases.contains(actor) {
                        *actor = own.local_id.clone();
                    }
                }
                reaction.participant_ids.sort();
                reaction.participant_ids.dedup();
                reaction.count = reaction.participant_ids.len() as u64;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use handover_core::*;
    #[test]
    fn self_aliases_are_scoped_and_merge_without_claiming_unknown_actors() {
        let id = ConversationId::new(MessagingAccountId::new("a"), "thread");
        let mut identities = ActorIdentities::default();
        identities.remember(
            id.clone(),
            Zeroizing::new(vec!["peer:sim-one".into(), "peer:sim-two".into()]),
        );
        let conversation = Conversation {
            id: id.clone(),
            kind: ConversationKind::Group,
            transport: TransportKind::Rcs,
            title: None,
            participants: vec![Participant {
                local_id: "self:a".into(),
                display_name: None,
                address: None,
                is_self: true,
            }],
            latest_message_id: None,
            last_activity_at: None,
            unread_count: None,
            cursor: None,
            capabilities: Default::default(),
        };
        let mut message = Message {
            id: MessageId::new(id, "m"),
            sender: conversation.participants[0].clone(),
            transport: None,
            sent_at: None,
            text: None,
            attachments: vec![],
            reply_preview: None,
            reply_to: None,
            deleted: false,
            reactions: vec![Reaction {
                emoji: "👍".into(),
                participant_ids: vec![
                    "peer:sim-one".into(),
                    "peer:sim-two".into(),
                    "peer:other".into(),
                ],
                count: 3,
            }],
        };
        identities.normalize(&conversation, std::slice::from_mut(&mut message));
        assert_eq!(
            message.reactions[0].participant_ids,
            ["peer:other", "self:a"]
        );
        assert_eq!(message.reactions[0].count, 2);
        let mut foreign = conversation.clone();
        foreign.id.account_id = MessagingAccountId::new("b");
        let mut copy = message.clone();
        copy.id.conversation_id = foreign.id.clone();
        copy.reactions[0].participant_ids = vec!["peer:sim-one".into()];
        copy.reactions[0].count = 1;
        identities.normalize(&foreign, std::slice::from_mut(&mut copy));
        assert_eq!(copy.reactions[0].participant_ids, ["peer:sim-one"]);
        identities.remember(conversation.id.clone(), Zeroizing::new(vec![]));
        message.reactions[0].participant_ids = vec!["peer:sim-one".into()];
        message.reactions[0].count = 1;
        identities.normalize(&conversation, std::slice::from_mut(&mut message));
        assert_eq!(message.reactions[0].participant_ids, ["peer:sim-one"]);
    }
}
