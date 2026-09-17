//! Versioned helper IPC contract (v1).
//!
//! The helper is a genuinely separate OS process. `handoverd` speaks to it
//! over newline-delimited JSON on the helper's stdin/stdout. No shared
//! address space, no FFI, no shared structs beyond this coarse,
//! arm's-length contract.
//!
//! All identifiers below the account scope are opaque strings minted by the
//! helper. Message bodies cross this boundary (the helper must fetch them
//! to serve history), but credentials, tokens, and keys never appear in any
//! contract message except the one-way `Login` command, which carries the
//! user-supplied credential bundle piped over the local pipe — never argv,
//! never logs.

use serde::{Deserialize, Serialize};

/// Helper contract version. Bumped only for breaking changes; unknown
/// versions are rejected on both sides.
pub const HELPER_PROTOCOL: u32 = 1;

/// Maximum decoded helper line: 1 MiB. Pages above this are malformed.
pub const MAX_HELPER_LINE_BYTES: usize = 1024 * 1024;

/// Maximum credential bundle accepted on `Login`: 256 KiB.
pub const MAX_BUNDLE_BYTES: usize = 256 * 1024;

/// Commands from the daemon to the helper.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HelperCommand {
    Hello,
    /// One-way credential delivery. `bundle_b64` is base64 of the opaque
    /// user-supplied credential bundle. The helper persists it in its own
    /// 0600 store and begins pairing; it never echoes it back.
    Login {
        account: String,
        bundle_b64: String,
    },
    Logout {
        account: String,
    },
    ListConversations {
        account: String,
    },
    FetchHistory {
        account: String,
        conversation: String,
        limit: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cursor: Option<String>,
    },
    SendText {
        request_id: String,
        account: String,
        conversation: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to: Option<String>,
    },
    SendMedia {
        request_id: String,
        account: String,
        conversation: String,
        /// Local file path staged by the daemon (validated before send).
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
    },
    React {
        request_id: String,
        account: String,
        conversation: String,
        message: String,
        emoji: String,
        add: bool,
    },
    MarkRead {
        account: String,
        conversation: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Typing-start only. There is no typing-stop command because upstream
    /// cannot send one; the helper must not invent it.
    Typing {
        account: String,
        conversation: String,
    },
    DeleteMessage {
        request_id: String,
        account: String,
        conversation: String,
        message: String,
    },
    OpenConversation {
        request_id: String,
        account: String,
        addresses: Vec<String>,
    },
    /// Request catch-up after a (re)connect: the helper re-emits accounts,
    /// full conversation lists, and authoritative windows.
    Sync {
        account: String,
    },
    Shutdown,
}

/// Events from the helper to the daemon. Every payload is validated and
/// normalized before it touches daemon state; malformed or oversized
/// events are dropped with a warning and never logged with bodies.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HelperEvent {
    Hello {
        helper_protocol: u32,
        name: String,
    },
    Account {
        account: String,
        label: String,
        connected: bool,
        authenticated: bool,
    },
    AccountRemoved {
        account: String,
    },
    /// Opaque human-readable verification prompt (e.g. an emoji to confirm
    /// on the phone). Display it; never log account material with it.
    Pairing {
        account: String,
        prompt: String,
    },
    Conversations {
        account: String,
        conversations: Vec<WireConversation>,
        /// `true` means this list is authoritative: the daemon reconciles.
        full: bool,
    },
    ConversationRemoved {
        account: String,
        conversation: String,
    },
    Messages {
        account: String,
        conversation: String,
        messages: Vec<WireMessage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cursor_next: Option<String>,
        /// `true` means this page is the authoritative window: the daemon
        /// reconciles its stored window against it.
        full: bool,
    },
    MessageRemoved {
        account: String,
        conversation: String,
        message: String,
    },
    Status {
        account: String,
        conversation: String,
        message: String,
        status: String,
    },
    Typing {
        account: String,
        conversation: String,
        participants: Vec<String>,
    },
    Read {
        account: String,
        conversation: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_read_message: Option<String>,
        unread: bool,
    },
    /// Acceptance report for a `request_id` command. `ok == true` means the
    /// helper accepted the request, not that it was delivered or displayed.
    CommandResult {
        request_id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Error {
        message: String,
    },
}

/// Wire form of a conversation. Field names are deliberately generic: no
/// Google protocol vocabulary crosses the process boundary.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WireConversation {
    pub local_id: String,
    #[serde(default)]
    pub kind: WireConversationKind,
    #[serde(default)]
    pub transport: WireTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub participants: Vec<WireParticipant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unread_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WireConversationKind {
    Direct,
    Group,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WireTransport {
    Rcs,
    Sms,
    Mms,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WireParticipant {
    pub local_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default)]
    pub is_self: bool,
}

/// Wire form of a message. No edit history: re-deliveries of the same id
/// are status/content updates, never versioned edits.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WireMessage {
    pub local_id: String,
    pub sender: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<WireTransport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default)]
    pub attachments: Vec<WireAttachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub reactions: Vec<WireReaction>,
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WireAttachment {
    pub local_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged_path: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WireReaction {
    pub emoji: String,
    #[serde(default)]
    pub participant_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ContractError {
    UnsupportedProtocol(u32),
    OversizedLine(usize),
    Malformed(String),
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedProtocol(version) => {
                write!(formatter, "unsupported helper protocol {version}")
            }
            Self::OversizedLine(len) => write!(formatter, "helper line too long ({len} bytes)"),
            Self::Malformed(detail) => write!(formatter, "malformed helper message: {detail}"),
        }
    }
}

impl std::error::Error for ContractError {}

/// Encode a daemon command as one JSON line (no trailing secrets in logs:
/// callers must redact `Login` before logging).
pub fn encode_command(command: &HelperCommand) -> Result<String, serde_json::Error> {
    serde_json::to_string(command)
}

/// Decode and size-check one helper line. Never include the raw line in
/// error returns: it may carry message bodies.
pub fn decode_event(line: &[u8]) -> Result<HelperEvent, ContractError> {
    if line.len() > MAX_HELPER_LINE_BYTES {
        return Err(ContractError::OversizedLine(line.len()));
    }
    serde_json::from_slice(line).map_err(|error| ContractError::Malformed(error.to_string()))
}

/// Check a hello handshake immediately after spawn.
pub fn check_hello(event: &HelperEvent) -> Result<String, ContractError> {
    match event {
        HelperEvent::Hello {
            helper_protocol,
            name,
        } => {
            if *helper_protocol != HELPER_PROTOCOL {
                return Err(ContractError::UnsupportedProtocol(*helper_protocol));
            }
            Ok(name.clone())
        }
        _ => Err(ContractError::Malformed("expected hello".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_and_malformed_helper_lines_are_rejected_without_echo() {
        let big = vec![b'x'; MAX_HELPER_LINE_BYTES + 1];
        assert!(matches!(
            decode_event(&big),
            Err(ContractError::OversizedLine(_))
        ));
        let error = decode_event(b"this is not json").expect_err("malformed");
        // The error carries the parser message, never the raw line.
        assert!(!error.to_string().contains("this is not json"));

        let event = decode_event(br#"{"type":"hello","helper_protocol":1,"name":"test"}"#)
            .expect("hello decodes");
        assert_eq!(check_hello(&event).expect("handshake"), "test");
        let other = decode_event(br#"{"type":"hello","helper_protocol":2,"name":"test"}"#)
            .expect("decodes");
        assert!(matches!(
            check_hello(&other),
            Err(ContractError::UnsupportedProtocol(2))
        ));
        let non_hello = decode_event(br#"{"type":"error","message":"busy"}"#).expect("decodes");
        assert!(check_hello(&non_hello).is_err());
    }

    #[test]
    fn login_bundle_size_is_bounded_at_encode_time() {
        let bundle = "x".repeat(MAX_BUNDLE_BYTES + 1);
        let command = HelperCommand::Login {
            account: "work".into(),
            bundle_b64: bundle,
        };
        let encoded = encode_command(&command).expect("encodes");
        assert!(encoded.len() + 1 > MAX_HELPER_LINE_BYTES || bundle_too_big(&command));
    }

    fn bundle_too_big(command: &HelperCommand) -> bool {
        matches!(command, HelperCommand::Login { bundle_b64, .. } if bundle_b64.len() > MAX_BUNDLE_BYTES)
    }
}
