//! `handover-gmessages-helper`: the separate helper process the daemon
//! supervises.
//!
//! License boundary: this binary is MIT and contains no Google companion
//! protocol code, no AGPL source, and no protobuf definitions. It speaks
//! the coarse JSON contract on stdin/stdout and delegates relay work to a
//! [`Relay`] implementation. The only bundled relay is [`LoopbackRelay`],
//! an in-memory stand-in for development and tests. A production relay
//! that drives the unmodified upstream bridge is operator-supplied and
//! lives outside this repository (see `docs/gmessages-sidecar.md`).
//!
//! The helper owns credential bundles (0600 files via [`secrets`]), pairing
//! ceremony state, relay RPCs, polling/recovery, and media transfer. It
//! never logs bundles, tokens, keys, message bodies, or media bytes.

use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, Write};
use std::path::PathBuf;

use handover_gmessages::contract::{
    HELPER_PROTOCOL, HelperCommand, HelperEvent, MAX_BUNDLE_BYTES, WireAttachment,
    WireConversation, WireConversationKind, WireMessage, WireParticipant, WireReaction,
    WireTransport,
};
use handover_gmessages::secrets;
use handover_gmessages::staging::{default_staging_directory, stage_upload};

const HELPER_NAME: &str = "handover-gmessages-helper/loopback";
const HISTORY_DEFAULT: usize = 20;

fn main() {
    let directory = secrets::default_directory()
        .unwrap_or_else(|_| std::env::temp_dir().join("handover-gmessages-fallback"));
    let mut helper = LoopbackHelper::new(directory);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let responses = helper.handle_line(line.as_bytes());
        for response in responses {
            stdout.write_all(response.as_bytes()).ok();
            stdout.write_all(b"\n").ok();
        }
        stdout.flush().ok();
        if helper.shutdown_requested() {
            break;
        }
    }
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let table = |c: u8| -> Result<u8, String> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err("invalid base64".into()),
        }
    };
    let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if clean.len() % 4 != 0 {
        return Err("invalid base64 length".into());
    }
    let mut output = Vec::with_capacity(clean.len() / 4 * 3);
    for chunk in clean.chunks(4) {
        let mut values = [0u8; 4];
        let mut padding = 0;
        for (index, byte) in chunk.iter().enumerate() {
            if *byte == b'=' {
                padding += 1;
            } else {
                if padding > 0 {
                    return Err("invalid base64 padding".into());
                }
                values[index] = table(*byte)?;
            }
        }
        output.push((values[0] << 2) | (values[1] >> 4));
        if padding < 2 {
            output.push((values[1] << 4) | (values[2] >> 2));
        }
        if padding < 1 {
            output.push((values[2] << 6) | values[3]);
        }
    }
    Ok(output)
}

/// A relay backend behind the helper boundary. Production relays implement
/// this trait against the upstream bridge process; the loopback fakes it
/// in memory. Returning `false` from a command means "not accepted"; the
/// helper reports that without inventing delivery state.
trait Relay {
    fn login(&mut self, account: &str, bundle: &[u8]) -> Vec<HelperEvent>;
    fn logout(&mut self, account: &str) -> Vec<HelperEvent>;
    fn sync(&mut self, account: &str) -> Vec<HelperEvent>;
    fn fetch_history(
        &mut self,
        account: &str,
        conversation: &str,
        limit: u32,
        cursor: Option<&str>,
    ) -> Vec<HelperEvent>;
    fn send_text(
        &mut self,
        request_id: &str,
        account: &str,
        conversation: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Vec<HelperEvent>;
    fn send_media(
        &mut self,
        request_id: &str,
        account: &str,
        conversation: &str,
        path: &str,
        caption: Option<&str>,
    ) -> Vec<HelperEvent>;
    fn react(
        &mut self,
        request_id: &str,
        account: &str,
        conversation: &str,
        message: &str,
        emoji: &str,
        add: bool,
    ) -> Vec<HelperEvent>;
    fn mark_read(
        &mut self,
        account: &str,
        conversation: &str,
        message: Option<&str>,
    ) -> Vec<HelperEvent>;
    fn typing(&mut self, account: &str, conversation: &str) -> Vec<HelperEvent>;
    fn delete_message(
        &mut self,
        request_id: &str,
        account: &str,
        conversation: &str,
        message: &str,
    ) -> Vec<HelperEvent>;
    fn open_conversation(
        &mut self,
        request_id: &str,
        account: &str,
        addresses: &[String],
    ) -> Vec<HelperEvent>;
}

struct StoredConversation {
    wire: WireConversation,
    /// Oldest first. Bounded in memory; the daemon owns the served window.
    messages: VecDeque<WireMessage>,
    read_unread: bool,
    read_last: Option<String>,
    /// Status progressions emitted on the next sync (loopback models the
    /// accepted -> sent -> delivered -> displayed pipeline honestly: each
    /// stage is a separate event, never assumed).
    pending_status: VecDeque<(String, String)>,
    counter: u64,
}

struct StoredAccount {
    label: String,
    authenticated: bool,
    conversations: BTreeMap<String, StoredConversation>,
}

struct LoopbackRelay {
    accounts: BTreeMap<String, StoredAccount>,
    media_sample: Option<PathBuf>,
    _media_staging: Option<tempfile::TempDir>,
}

impl LoopbackRelay {
    fn new() -> Self {
        let mut relay = Self {
            accounts: BTreeMap::new(),
            media_sample: None,
            _media_staging: None,
        };
        // One real staged file so inbound attachment paths validate.
        if let Ok(root) = default_staging_directory() {
            if std::fs::create_dir_all(&root).is_ok() {
                if let Ok(staging) = tempfile::Builder::new()
                    .prefix("loopback-")
                    .tempdir_in(root)
                {
                    let path = staging.path().join("sample.bin");
                    if std::fs::write(&path, b"loopback sample attachment").is_ok() {
                        relay.media_sample = Some(path);
                        relay._media_staging = Some(staging);
                    }
                }
            }
        }
        relay
    }

    fn account_mut(&mut self, account: &str) -> Option<&mut StoredAccount> {
        self.accounts.get_mut(account)
    }

    fn ensure_account(&mut self, account: &str) {
        self.accounts
            .entry(account.into())
            .or_insert(StoredAccount {
                label: format!("Loopback {account}"),
                authenticated: false,
                conversations: BTreeMap::new(),
            });
    }

    fn seed(&mut self, account: &str) {
        let stored = match self.accounts.get_mut(account) {
            Some(stored) => stored,
            None => return,
        };
        if !stored.conversations.is_empty() {
            return;
        }
        let full_caps = vec![
            "text".into(),
            "media".into(),
            "reactions".into(),
            "replies".into(),
            "read_receipts".into(),
            "typing_send".into(),
            "typing_receive".into(),
            "message_delete_own".into(),
            "attachment_download".into(),
        ];
        let mut rcs = StoredConversation {
            wire: WireConversation {
                local_id: "thread-rcs".into(),
                kind: WireConversationKind::Direct,
                transport: WireTransport::Rcs,
                title: None,
                participants: vec![
                    WireParticipant {
                        local_id: "self".into(),
                        display_name: Some("Me".into()),
                        address: Some("+15550000".into()),
                        is_self: true,
                    },
                    WireParticipant {
                        local_id: "peer".into(),
                        display_name: Some("RCS Friend".into()),
                        address: Some("+15550001".into()),
                        is_self: false,
                    },
                ],
                latest_message: Some("m3".into()),
                last_activity_at: None,
                unread_count: Some(1),
                cursor: Some("m1".into()),
                capabilities: full_caps.clone(),
            },
            messages: VecDeque::from([
                text_message("m1", "peer", 1_758_000_000_000_000, "Hey, RCS works here"),
                text_message("m2", "self", 1_758_000_001_000_000, "Hello from Linux"),
                text_message("m3", "peer", 1_758_000_002_000_000, "Nice"),
            ]),
            read_unread: true,
            read_last: Some("m2".into()),
            pending_status: VecDeque::new(),
            counter: 100,
        };
        if let Some(sample) = self.media_sample.as_ref().and_then(|p| p.to_str()) {
            rcs.messages.push_back(WireMessage {
                local_id: "m4".into(),
                sender: "peer".into(),
                transport: Some(WireTransport::Rcs),
                sent_at: Some(1_758_000_003_000_000),
                text: Some("A file for you".into()),
                attachments: vec![WireAttachment {
                    local_id: "a1".into(),
                    mime: Some("application/octet-stream".into()),
                    name: Some("sample.bin".into()),
                    size_bytes: Some(25),
                    staged_path: Some(sample.into()),
                }],
                reply_to: None,
                reactions: vec![],
                deleted: false,
            });
            rcs.wire.latest_message = Some("m4".into());
        }
        stored.conversations.insert("thread-rcs".into(), rcs);
        stored.conversations.insert(
            "thread-sms".into(),
            StoredConversation {
                wire: WireConversation {
                    local_id: "thread-sms".into(),
                    kind: WireConversationKind::Direct,
                    transport: WireTransport::Sms,
                    title: None,
                    participants: vec![
                        WireParticipant {
                            local_id: "self".into(),
                            display_name: Some("Me".into()),
                            address: Some("+15550000".into()),
                            is_self: true,
                        },
                        WireParticipant {
                            local_id: "sms-peer".into(),
                            display_name: Some("SMS Friend".into()),
                            address: Some("+15550002".into()),
                            is_self: false,
                        },
                    ],
                    latest_message: Some("s1".into()),
                    last_activity_at: None,
                    unread_count: Some(0),
                    cursor: None,
                    capabilities: vec!["text".into()],
                },
                messages: VecDeque::from([text_message(
                    "s1",
                    "sms-peer",
                    1_758_000_010_000_000,
                    "SMS fallback thread",
                )]),
                read_unread: false,
                read_last: Some("s1".into()),
                pending_status: VecDeque::new(),
                counter: 100,
            },
        );
        stored.conversations.insert(
            "thread-group".into(),
            StoredConversation {
                wire: WireConversation {
                    local_id: "thread-group".into(),
                    kind: WireConversationKind::Group,
                    transport: WireTransport::Rcs,
                    title: Some("Weekend plans".into()),
                    participants: vec![
                        WireParticipant {
                            local_id: "self".into(),
                            display_name: Some("Me".into()),
                            address: Some("+15550000".into()),
                            is_self: true,
                        },
                        WireParticipant {
                            local_id: "peer".into(),
                            display_name: Some("RCS Friend".into()),
                            address: Some("+15550001".into()),
                            is_self: false,
                        },
                        WireParticipant {
                            local_id: "peer2".into(),
                            display_name: Some("Second Friend".into()),
                            address: Some("+15550003".into()),
                            is_self: false,
                        },
                    ],
                    latest_message: Some("g1".into()),
                    last_activity_at: None,
                    unread_count: Some(0),
                    cursor: None,
                    capabilities: {
                        let mut caps = full_caps;
                        caps.push("group_create".into());
                        caps.push("conversation_delete".into());
                        caps
                    },
                },
                messages: VecDeque::from([text_message(
                    "g1",
                    "peer2",
                    1_758_000_020_000_000,
                    "Saturday?",
                )]),
                read_unread: false,
                read_last: Some("g1".into()),
                pending_status: VecDeque::new(),
                counter: 100,
            },
        );
    }

    fn full_window(&self, stored: &StoredConversation) -> Vec<WireMessage> {
        stored
            .messages
            .iter()
            .rev()
            .take(HISTORY_DEFAULT)
            .rev()
            .cloned()
            .collect()
    }

    fn page(
        stored: &StoredConversation,
        limit: usize,
        cursor: Option<&str>,
    ) -> (Vec<WireMessage>, Option<String>) {
        let messages: Vec<&WireMessage> = stored.messages.iter().collect();
        let end = match cursor {
            None => messages.len(),
            Some(cursor) => messages
                .iter()
                .position(|message| message.local_id == cursor)
                .unwrap_or(messages.len()),
        };
        let start = end.saturating_sub(limit.max(1));
        let page: Vec<WireMessage> = messages[start..end].iter().map(|m| (*m).clone()).collect();
        let next = if start > 0 {
            page.first().map(|message| message.local_id.clone())
        } else {
            None
        };
        (page, next)
    }

    fn result(request_id: &str, ok: bool, error: Option<&str>) -> HelperEvent {
        HelperEvent::CommandResult {
            request_id: request_id.into(),
            ok,
            error: error.map(str::to_string),
        }
    }
}

fn text_message(local_id: &str, sender: &str, sent_at: i64, text: &str) -> WireMessage {
    WireMessage {
        local_id: local_id.into(),
        sender: sender.into(),
        transport: Some(WireTransport::Rcs),
        sent_at: Some(sent_at),
        text: Some(text.into()),
        attachments: vec![],
        reply_to: None,
        reactions: vec![],
        deleted: false,
    }
}

impl Relay for LoopbackRelay {
    fn login(&mut self, account: &str, bundle: &[u8]) -> Vec<HelperEvent> {
        if bundle.is_empty() {
            return vec![HelperEvent::Error {
                message: format!("login rejected for {account}: empty bundle"),
            }];
        }
        self.accounts
            .entry(account.into())
            .or_insert(StoredAccount {
                label: format!("Loopback {account}"),
                authenticated: false,
                conversations: BTreeMap::new(),
            });
        vec![
            HelperEvent::Account {
                account: account.into(),
                label: format!("Loopback {account}"),
                connected: true,
                authenticated: false,
            },
            // Opaque verification prompt. A production relay emits the real
            // pairing signal here (e.g. the emoji to confirm on the phone);
            // the loopback models the ceremony as a pending confirmation
            // that completes on the next sync.
            HelperEvent::Pairing {
                account: account.into(),
                prompt: "Confirm the pending pairing on the phone, then sync again".into(),
            },
        ]
    }

    fn logout(&mut self, account: &str) -> Vec<HelperEvent> {
        self.accounts.remove(account);
        vec![HelperEvent::AccountRemoved {
            account: account.into(),
        }]
    }

    fn sync(&mut self, account: &str) -> Vec<HelperEvent> {
        // Flip pairing-pending accounts to authenticated (models the user
        // confirming on the phone between login and this sync).
        if let Some(stored) = self.accounts.get_mut(account) {
            stored.authenticated = true;
        }
        self.seed(account);
        let mut events = Vec::new();
        let Some(stored) = self.accounts.get(account) else {
            return vec![HelperEvent::AccountRemoved {
                account: account.into(),
            }];
        };
        events.push(HelperEvent::Account {
            account: account.into(),
            label: stored.label.clone(),
            connected: true,
            authenticated: stored.authenticated,
        });
        events.push(HelperEvent::Conversations {
            account: account.into(),
            conversations: stored
                .conversations
                .values()
                .map(|conversation| conversation.wire.clone())
                .collect(),
            full: true,
            generation: None,
        });
        for (local_id, conversation) in &stored.conversations {
            events.push(HelperEvent::Messages {
                account: account.into(),
                conversation: local_id.clone(),
                messages: self.full_window(conversation),
                cursor_next: None,
                page_complete: false,
                full: true,
                generation: None,
                fetch_id: None,
            });
            events.push(HelperEvent::Read {
                account: account.into(),
                conversation: local_id.clone(),
                last_read_message: conversation.read_last.clone(),
                unread: conversation.read_unread,
            });
        }
        // Drain one pending status stage per conversation per sync: stages
        // are attested one at a time, never fast-forwarded.
        let pending: Vec<(String, String, String)> = self
            .accounts
            .get_mut(account)
            .map(|stored| {
                stored
                    .conversations
                    .iter_mut()
                    .filter_map(|(local_id, conversation)| {
                        conversation
                            .pending_status
                            .pop_front()
                            .map(|(message, status)| (local_id.clone(), message, status))
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (conversation, message, status) in pending {
            events.push(HelperEvent::Status {
                account: account.into(),
                conversation,
                message,
                status,
            });
        }
        events
    }

    fn fetch_history(
        &mut self,
        account: &str,
        conversation: &str,
        limit: u32,
        cursor: Option<&str>,
    ) -> Vec<HelperEvent> {
        let limit = limit.clamp(1, 100) as usize;
        let Some(stored) = self.account_mut(account) else {
            return vec![HelperEvent::AccountRemoved {
                account: account.into(),
            }];
        };
        let Some(thread) = stored.conversations.get(conversation) else {
            return vec![HelperEvent::Error {
                message: format!("unknown conversation {account}:{conversation}"),
            }];
        };
        let (page, next) = Self::page(thread, limit, cursor);
        vec![HelperEvent::Messages {
            account: account.into(),
            conversation: conversation.into(),
            messages: page,
            cursor_next: next,
            page_complete: true,
            full: false,
            generation: None,
            fetch_id: None,
        }]
    }

    fn send_text(
        &mut self,
        request_id: &str,
        account: &str,
        conversation: &str,
        text: &str,
        _reply_to: Option<&str>,
    ) -> Vec<HelperEvent> {
        let Some(stored) = self.account_mut(account) else {
            return vec![Self::result(request_id, false, Some("unknown account"))];
        };
        let Some(thread) = stored.conversations.get_mut(conversation) else {
            return vec![Self::result(
                request_id,
                false,
                Some("unknown conversation"),
            )];
        };
        if !thread
            .wire
            .capabilities
            .iter()
            .any(|capability| capability == "text")
        {
            return vec![Self::result(request_id, false, Some("text not attested"))];
        }
        thread.counter += 1;
        let local_id = format!("out-{}", thread.counter);
        thread.messages.push_back(WireMessage {
            local_id: local_id.clone(),
            sender: "self".into(),
            transport: Some(WireTransport::Rcs),
            sent_at: Some(1_758_000_100_000_000 + thread.counter as i64),
            text: Some(text.into()),
            attachments: vec![],
            reply_to: None,
            reactions: vec![],
            deleted: false,
        });
        thread.wire.latest_message = Some(local_id.clone());
        // Accepted now; later stages attest one per sync.
        thread
            .pending_status
            .push_back((local_id.clone(), "sent".into()));
        thread
            .pending_status
            .push_back((local_id.clone(), "delivered".into()));
        thread
            .pending_status
            .push_back((local_id.clone(), "displayed".into()));
        vec![
            Self::result(request_id, true, None),
            HelperEvent::Status {
                account: account.into(),
                conversation: conversation.into(),
                message: local_id.clone(),
                status: "accepted".into(),
            },
            HelperEvent::Messages {
                account: account.into(),
                conversation: conversation.into(),
                messages: vec![thread.messages.back().cloned().expect("just pushed")],
                cursor_next: None,
                page_complete: false,
                full: false,
                generation: None,
                fetch_id: None,
            },
        ]
    }

    fn send_media(
        &mut self,
        request_id: &str,
        account: &str,
        conversation: &str,
        path: &str,
        caption: Option<&str>,
    ) -> Vec<HelperEvent> {
        let metadata = std::fs::metadata(path);
        let size = metadata.map(|metadata| metadata.len()).unwrap_or(0);
        if size == 0 {
            return vec![Self::result(request_id, false, Some("unreadable file"))];
        }
        let staged_path = self._media_staging.as_ref().and_then(|directory| {
            stage_upload(PathBuf::from(path).as_path(), directory.path()).ok()
        });
        let Some(staged_path) = staged_path else {
            return vec![Self::result(
                request_id,
                false,
                Some("unable to stage file"),
            )];
        };
        let Some(stored) = self.account_mut(account) else {
            return vec![Self::result(request_id, false, Some("unknown account"))];
        };
        let Some(thread) = stored.conversations.get_mut(conversation) else {
            return vec![Self::result(
                request_id,
                false,
                Some("unknown conversation"),
            )];
        };
        if !thread
            .wire
            .capabilities
            .iter()
            .any(|capability| capability == "media")
        {
            return vec![Self::result(request_id, false, Some("media not attested"))];
        }
        thread.counter += 1;
        let local_id = format!("out-{}", thread.counter);
        let name = PathBuf::from(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("attachment")
            .to_string();
        thread.messages.push_back(WireMessage {
            local_id: local_id.clone(),
            sender: "self".into(),
            transport: Some(WireTransport::Sms),
            sent_at: Some(1_758_000_100_000_000 + thread.counter as i64),
            text: caption.map(str::to_string),
            attachments: vec![WireAttachment {
                local_id: format!("{local_id}-a1"),
                mime: Some("application/octet-stream".into()),
                name: Some(name),
                size_bytes: Some(size),
                staged_path: Some(staged_path.to_string_lossy().into_owned()),
            }],
            reply_to: None,
            reactions: vec![],
            deleted: false,
        });
        thread.wire.latest_message = Some(local_id.clone());
        thread
            .pending_status
            .push_back((local_id.clone(), "sent".into()));
        thread
            .pending_status
            .push_back((local_id.clone(), "delivered".into()));
        vec![
            Self::result(request_id, true, None),
            HelperEvent::Status {
                account: account.into(),
                conversation: conversation.into(),
                message: local_id.clone(),
                status: "accepted".into(),
            },
            HelperEvent::Messages {
                account: account.into(),
                conversation: conversation.into(),
                messages: vec![thread.messages.back().cloned().expect("just pushed")],
                cursor_next: None,
                page_complete: false,
                full: false,
                generation: None,
                fetch_id: None,
            },
        ]
    }

    fn react(
        &mut self,
        request_id: &str,
        account: &str,
        conversation: &str,
        message: &str,
        emoji: &str,
        add: bool,
    ) -> Vec<HelperEvent> {
        let Some(stored) = self.account_mut(account) else {
            return vec![Self::result(request_id, false, Some("unknown account"))];
        };
        let Some(thread) = stored.conversations.get_mut(conversation) else {
            return vec![Self::result(
                request_id,
                false,
                Some("unknown conversation"),
            )];
        };
        let Some(target) = thread
            .messages
            .iter_mut()
            .find(|candidate| candidate.local_id == message)
        else {
            return vec![Self::result(request_id, false, Some("unknown message"))];
        };
        if add {
            if let Some(existing) = target
                .reactions
                .iter_mut()
                .find(|reaction| reaction.emoji == emoji)
            {
                if !existing.participant_ids.iter().any(|id| id == "self") {
                    existing.participant_ids.push("self".into());
                }
            } else {
                target.reactions.push(WireReaction {
                    emoji: emoji.into(),
                    participant_ids: vec!["self".into()],
                });
            }
        } else {
            target.reactions.retain_mut(|reaction| {
                if reaction.emoji != emoji {
                    return true;
                }
                reaction.participant_ids.retain(|id| id != "self");
                !reaction.participant_ids.is_empty()
            });
        }
        let updated = target.clone();
        vec![
            Self::result(request_id, true, None),
            HelperEvent::Messages {
                account: account.into(),
                conversation: conversation.into(),
                messages: vec![updated],
                cursor_next: None,
                page_complete: false,
                full: false,
                generation: None,
                fetch_id: None,
            },
        ]
    }

    fn mark_read(
        &mut self,
        account: &str,
        conversation: &str,
        message: Option<&str>,
    ) -> Vec<HelperEvent> {
        let Some(stored) = self.account_mut(account) else {
            return vec![HelperEvent::Error {
                message: format!("unknown account {account}"),
            }];
        };
        let Some(thread) = stored.conversations.get_mut(conversation) else {
            return vec![HelperEvent::Error {
                message: format!("unknown conversation {account}:{conversation}"),
            }];
        };
        if let Some(message) = message {
            thread.read_last = Some(message.into());
        } else {
            thread.read_last = thread.messages.back().map(|last| last.local_id.clone());
        }
        thread.read_unread = false;
        thread.wire.unread_count = Some(0);
        vec![HelperEvent::Read {
            account: account.into(),
            conversation: conversation.into(),
            last_read_message: thread.read_last.clone(),
            unread: false,
        }]
    }

    fn typing(&mut self, account: &str, conversation: &str) -> Vec<HelperEvent> {
        // Inbound typing indicator from the peer (loopback simulates the
        // remote party typing after we announce our own typing-start).
        vec![HelperEvent::Typing {
            account: account.into(),
            conversation: conversation.into(),
            participants: vec!["peer".into()],
        }]
    }

    fn delete_message(
        &mut self,
        request_id: &str,
        account: &str,
        conversation: &str,
        message: &str,
    ) -> Vec<HelperEvent> {
        let Some(stored) = self.account_mut(account) else {
            return vec![Self::result(request_id, false, Some("unknown account"))];
        };
        let Some(thread) = stored.conversations.get_mut(conversation) else {
            return vec![Self::result(
                request_id,
                false,
                Some("unknown conversation"),
            )];
        };
        let position = thread
            .messages
            .iter()
            .position(|candidate| candidate.local_id == message);
        let Some(position) = position else {
            return vec![Self::result(request_id, false, Some("unknown message"))];
        };
        if thread.messages[position].sender != "self" {
            return vec![Self::result(request_id, false, Some("not own message"))];
        }
        thread.messages.remove(position);
        vec![
            Self::result(request_id, true, None),
            HelperEvent::MessageRemoved {
                account: account.into(),
                conversation: conversation.into(),
                message: message.into(),
            },
        ]
    }

    fn open_conversation(
        &mut self,
        request_id: &str,
        account: &str,
        addresses: &[String],
    ) -> Vec<HelperEvent> {
        if addresses.is_empty() {
            return vec![Self::result(request_id, false, Some("no addresses"))];
        }
        self.seed(account);
        let Some(stored) = self.account_mut(account) else {
            return vec![Self::result(request_id, false, Some("unknown account"))];
        };
        // Resolve an existing direct thread by address before creating.
        if addresses.len() == 1 {
            let hit = stored
                .conversations
                .iter()
                .find(|(_, thread)| {
                    thread.wire.kind == WireConversationKind::Direct
                        && thread.wire.participants.iter().any(|participant| {
                            participant.address.as_deref() == Some(addresses[0].as_str())
                        })
                })
                .map(|(local_id, _)| local_id.clone());
            if let Some(local_id) = hit {
                return vec![
                    Self::result(request_id, true, None),
                    HelperEvent::Messages {
                        account: account.into(),
                        conversation: local_id,
                        messages: vec![],
                        cursor_next: None,
                        page_complete: false,
                        full: false,
                        generation: None,
                        fetch_id: None,
                    },
                ];
            }
        }
        let local_id = format!("thread-open-{}", stored.conversations.len() + 1);
        let mut participants = vec![WireParticipant {
            local_id: "self".into(),
            display_name: Some("Me".into()),
            address: Some("+15550000".into()),
            is_self: true,
        }];
        for (index, address) in addresses.iter().enumerate() {
            participants.push(WireParticipant {
                local_id: format!("new-{index}"),
                display_name: None,
                address: Some(address.clone()),
                is_self: false,
            });
        }
        let kind = if addresses.len() == 1 {
            WireConversationKind::Direct
        } else {
            WireConversationKind::Group
        };
        stored.conversations.insert(
            local_id.clone(),
            StoredConversation {
                wire: WireConversation {
                    local_id: local_id.clone(),
                    kind,
                    transport: WireTransport::Rcs,
                    title: None,
                    participants,
                    latest_message: None,
                    last_activity_at: None,
                    unread_count: Some(0),
                    cursor: None,
                    capabilities: vec![
                        "text".into(),
                        "media".into(),
                        "reactions".into(),
                        "replies".into(),
                        "read_receipts".into(),
                        "typing_send".into(),
                        "typing_receive".into(),
                        "message_delete_own".into(),
                        "attachment_download".into(),
                    ],
                },
                messages: VecDeque::new(),
                read_unread: false,
                read_last: None,
                pending_status: VecDeque::new(),
                counter: 100,
            },
        );
        let wire = stored.conversations[&local_id].wire.clone();
        vec![
            Self::result(request_id, true, None),
            HelperEvent::Conversations {
                account: account.into(),
                conversations: vec![wire],
                full: false,
                generation: None,
            },
        ]
    }
}

struct LoopbackHelper {
    relay: LoopbackRelay,
    directory: PathBuf,
    shutdown: bool,
}

impl LoopbackHelper {
    fn new(directory: PathBuf) -> Self {
        Self {
            relay: LoopbackRelay::new(),
            directory,
            shutdown: false,
        }
    }

    fn shutdown_requested(&self) -> bool {
        self.shutdown
    }

    fn handle_line(&mut self, line: &[u8]) -> Vec<String> {
        let command: HelperCommand = match serde_json::from_slice(line) {
            Ok(command) => command,
            Err(_) => {
                return vec![encode(&HelperEvent::Error {
                    message: "malformed command".into(),
                })];
            }
        };
        // Bundle size is enforced before decode cost grows.
        if let HelperCommand::Login { bundle_b64, .. } = &command {
            if bundle_b64.len() > MAX_BUNDLE_BYTES {
                return vec![encode(&HelperEvent::Error {
                    message: "credential bundle too large".into(),
                })];
            }
        }
        let events = self.dispatch(command);
        events.iter().map(encode).collect()
    }

    fn dispatch(&mut self, command: HelperCommand) -> Vec<HelperEvent> {
        match command {
            HelperCommand::Hello => {
                // Session restore: every persisted credential bundle gets an
                // account announcement so a restarted daemon recovers without
                // asking the user to pair again. Pairing ceremony state
                // itself is re-attested through the normal sync flow.
                let mut events = vec![HelperEvent::Hello {
                    helper_protocol: HELPER_PROTOCOL,
                    name: HELPER_NAME.into(),
                }];
                if let Ok(entries) = std::fs::read_dir(&self.directory) {
                    let mut accounts: Vec<String> = entries
                        .filter_map(|entry| entry.ok())
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                        .filter(|name| name.ends_with(".credentials.json"))
                        .map(|name| name.trim_end_matches(".credentials.json").to_string())
                        .collect();
                    accounts.sort();
                    for account in accounts {
                        if !account.is_empty() && account.len() <= 128 {
                            self.relay.ensure_account(&account);
                            events.push(HelperEvent::Account {
                                account: account.clone(),
                                label: format!("Loopback {account}"),
                                connected: true,
                                authenticated: false,
                            });
                        }
                    }
                }
                events
            }
            HelperCommand::Login {
                account,
                bundle_b64,
            } => {
                match decode_base64(&bundle_b64) {
                    Ok(bundle) => {
                        if bundle.is_empty() || bundle.len() > MAX_BUNDLE_BYTES {
                            return vec![HelperEvent::Error {
                                message: format!("login rejected for {account}: bad bundle"),
                            }];
                        }
                        // Persist before acting: a crash after ack must not
                        // lose the credential the user just supplied.
                        if let Err(error) =
                            secrets::store_bundle(&self.directory, &account, &bundle)
                        {
                            return vec![HelperEvent::Error {
                                message: format!("login storage failed for {account}: {error}"),
                            }];
                        }
                        self.relay.login(&account, &bundle)
                    }
                    Err(_) => vec![HelperEvent::Error {
                        message: format!("login rejected for {account}: bad encoding"),
                    }],
                }
            }
            HelperCommand::Logout { account } => {
                let _ = secrets::delete_bundle(&self.directory, &account);
                self.relay.logout(&account)
            }
            HelperCommand::ListConversations { account } => self.relay.sync(&account),
            HelperCommand::FetchHistory {
                account,
                conversation,
                limit,
                cursor,
                fetch_id,
            } => self
                .relay
                .fetch_history(&account, &conversation, limit, cursor.as_deref())
                .into_iter()
                .map(|event| match event {
                    HelperEvent::Messages {
                        account,
                        conversation,
                        messages,
                        cursor_next,
                        page_complete,
                        full,
                        generation,
                        ..
                    } => HelperEvent::Messages {
                        account,
                        conversation,
                        messages,
                        cursor_next,
                        page_complete,
                        full,
                        generation,
                        fetch_id,
                    },
                    other => other,
                })
                .collect(),
            HelperCommand::SendText {
                request_id,
                account,
                conversation,
                text,
                reply_to,
            } => self.relay.send_text(
                &request_id,
                &account,
                &conversation,
                &text,
                reply_to.as_deref(),
            ),
            HelperCommand::SendMedia {
                request_id,
                account,
                conversation,
                path,
                caption,
            } => self.relay.send_media(
                &request_id,
                &account,
                &conversation,
                &path,
                caption.as_deref(),
            ),
            HelperCommand::React {
                request_id,
                account,
                conversation,
                message,
                emoji,
                add,
            } => self
                .relay
                .react(&request_id, &account, &conversation, &message, &emoji, add),
            HelperCommand::MarkRead {
                account,
                conversation,
                message,
            } => self
                .relay
                .mark_read(&account, &conversation, message.as_deref()),
            HelperCommand::Typing {
                account,
                conversation,
            } => self.relay.typing(&account, &conversation),
            HelperCommand::DeleteMessage {
                request_id,
                account,
                conversation,
                message,
            } => self
                .relay
                .delete_message(&request_id, &account, &conversation, &message),
            HelperCommand::OpenConversation {
                request_id,
                account,
                addresses,
            } => self
                .relay
                .open_conversation(&request_id, &account, &addresses),
            HelperCommand::Sync { account } => self.relay.sync(&account),
            HelperCommand::Shutdown => {
                self.shutdown = true;
                vec![]
            }
        }
    }
}

fn encode(event: &HelperEvent) -> String {
    serde_json::to_string(event).expect("helper events serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip_without_external_crate() {
        // " OPAQUE-BUNDLE:42/+" exercises padding and the full alphabet.
        let decoded = decode_base64("IE9QQVFVRS1CVU5ETEU6NDIvKw==").expect("decode");
        assert_eq!(decoded, b" OPAQUE-BUNDLE:42/+".to_vec());
        assert!(decode_base64("!!!").is_err());
        assert!(decode_base64("abc").is_err());
    }
}
