//! Daemon-side helper supervision.
//!
//! The helper is optional: when no helper binary is configured or found,
//! messaging is simply unavailable and every other subsystem keeps working.
//! A crashing or misbehaving helper marks its accounts disconnected; it can
//! never disturb native battery/notifications/media/sharing.
//!
//! Restart policy is bounded exponential backoff with jitter-free steps
//! (1s, 2s, 4s, … capped at 60s). After every (re)connect the daemon sends
//! `Sync` per authenticated account so the helper re-emits authoritative
//! state; local windows reconcile instead of accumulating duplicates.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

/// Upper bound for the spawn hello exchange. A helper that starts but
/// never answers must fail fast instead of stalling supervision.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
/// Upper bound for one stdin write. A helper that stops reading must
/// be restarted, not waited on forever.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(10);

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::contract::{
    HelperCommand, HelperEvent, MAX_HELPER_LINE_BYTES, check_hello, decode_event, encode_command,
};

/// Environment override for the helper binary path. Never carries secrets.
pub const HELPER_ENV: &str = "HANDOVER_GMESSAGES_HELPER";
/// Native helper shipped with the daemon.
pub const HELPER_BINARY: &str = "handover-google-messages-helper";
/// Optional compatibility helper for installations without the native binary.
pub const LEGACY_HELPER_BINARY: &str = "handover-gmessages";

/// Prefer an explicit override, then the bundled native helper, then PATH.
/// Legacy discovery applies only when the native binary is absent. A failed
/// native session never silently changes providers or retries a send elsewhere.
/// Returns `None` when messaging should stay dormant.
pub fn find_helper() -> Option<PathBuf> {
    let configured = std::env::var_os(HELPER_ENV);
    let executable = std::env::current_exe().ok();
    let paths = std::env::var_os("PATH");
    find_helper_in(
        configured.as_deref(),
        executable.as_deref().and_then(Path::parent),
        paths.as_deref(),
    )
}

fn find_helper_in(
    configured: Option<&std::ffi::OsStr>,
    binary_directory: Option<&Path>,
    paths: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    if let Some(path) = configured {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
        return None;
    }
    if let Some(directory) = binary_directory {
        let bundled = directory.join(HELPER_BINARY);
        if bundled.is_file() {
            return Some(bundled);
        }
    }
    let paths = paths?;
    for name in [HELPER_BINARY, LEGACY_HELPER_BINARY] {
        if let Some(path) = std::env::split_paths(paths)
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
        {
            return Some(path);
        }
    }
    None
}

pub fn backoff_delay(restarts: u32) -> Duration {
    let shift = restarts.min(6);
    Duration::from_secs((1u64 << shift).min(60))
}

#[derive(Debug)]
pub enum SpawnError {
    Io(std::io::Error),
    Handshake(crate::contract::ContractError),
    NoOutput,
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "helper spawn failed: {error}"),
            Self::Handshake(error) => write!(formatter, "helper handshake failed: {error}"),
            Self::NoOutput => write!(formatter, "helper exited before handshake"),
        }
    }
}

impl std::error::Error for SpawnError {}

pub struct HelperProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: EventReader<BufReader<ChildStdout>>,
    pub name: String,
}

/// Persistent framing state for the helper's newline-delimited event stream.
/// `next_event` is cancellation-safe: bytes consumed from the underlying
/// reader remain in `line` or `discarded` if the future is dropped.
struct EventReader<R> {
    reader: R,
    line: Vec<u8>,
    discarding: bool,
    discarded: usize,
}

impl<R> EventReader<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            line: Vec::new(),
            discarding: false,
            discarded: 0,
        }
    }

    fn reset(&mut self) {
        self.line.clear();
        self.discarding = false;
        self.discarded = 0;
    }
}

impl<R: AsyncBufRead + Unpin> EventReader<R> {
    async fn next_event(&mut self) -> Result<Option<HelperEvent>, SpawnError> {
        const MAX_DISCARD_BYTES: usize = 10 * 1024 * 1024;
        loop {
            let available = self.reader.fill_buf().await.map_err(SpawnError::Io)?;
            if available.is_empty() {
                if self.discarding {
                    let size = self.discarded;
                    self.reset();
                    return Err(SpawnError::Handshake(
                        crate::contract::ContractError::OversizedLine(size),
                    ));
                }
                if self.line.is_empty() {
                    return Ok(None);
                }
                break;
            }

            let newline = available.iter().position(|byte| *byte == b'\n');
            let consumed = newline.map_or(available.len(), |position| position + 1);
            let content_length = newline.unwrap_or(available.len());
            // The limit applies to the complete wire line, including its
            // newline, matching outbound framing and the helper contract.
            let wire_length = content_length + usize::from(newline.is_some());
            if self.discarding {
                self.discarded = self.discarded.saturating_add(wire_length);
            } else if self.line.len().saturating_add(wire_length) > MAX_HELPER_LINE_BYTES {
                self.discarding = true;
                self.discarded = self.line.len().saturating_add(wire_length);
                self.line.clear();
            } else {
                self.line.extend_from_slice(&available[..content_length]);
            }
            self.reader.consume(consumed);

            if self.discarded > MAX_DISCARD_BYTES {
                // The record never terminated. Restart the helper rather than
                // treating its remaining bytes as a new event.
                return Err(SpawnError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "helper event exceeded discard limit",
                )));
            }
            if newline.is_some() {
                if self.discarding {
                    let size = self.discarded;
                    self.reset();
                    return Err(SpawnError::Handshake(
                        crate::contract::ContractError::OversizedLine(size),
                    ));
                }
                break;
            }
        }

        let line = std::mem::take(&mut self.line);
        self.reset();
        decode_event(&line).map(Some).map_err(SpawnError::Handshake)
    }
}

impl HelperProcess {
    pub async fn spawn(path: &Path) -> Result<Self, SpawnError> {
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(SpawnError::Io)?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SpawnError::Io(std::io::Error::other("helper stdin unavailable")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SpawnError::Io(std::io::Error::other("helper stdout unavailable")))?;
        let mut process = Self {
            child,
            stdin,
            stdout: EventReader::new(BufReader::new(stdout)),
            name: String::new(),
        };
        // A started-but-hung helper must not stall the supervisor
        // forever: bound the hello exchange. The child is killed on
        // drop, so a timeout always cleans up.
        let event = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
            process.send(&HelperCommand::Hello).await?;
            process.next_event().await?.ok_or(SpawnError::NoOutput)
        })
        .await
        .map_err(|_| {
            SpawnError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "helper handshake timed out",
            ))
        })??;
        process.name = check_hello(&event).map_err(SpawnError::Handshake)?;
        Ok(process)
    }

    pub async fn send(&mut self, command: &HelperCommand) -> Result<(), SpawnError> {
        // Redact credential bundles before any error can echo them.
        let encoded = encode_command(command).map_err(|error| {
            SpawnError::Handshake(crate::contract::ContractError::Malformed(error.to_string()))
        })?;
        if encoded.len() + 1 > MAX_HELPER_LINE_BYTES {
            return Err(SpawnError::Handshake(
                crate::contract::ContractError::OversizedLine(encoded.len() + 1),
            ));
        }
        // A helper that stops reading stdin would otherwise fill its
        // pipe and wedge every outbound command. Time the write out
        // and kill the child so supervision restarts it.
        let result = tokio::time::timeout(SEND_TIMEOUT, async {
            self.stdin.write_all(encoded.as_bytes()).await?;
            self.stdin.write_all(b"\n").await?;
            self.stdin.flush().await
        })
        .await;
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(SpawnError::Io(error)),
            Err(_) => {
                let _ = self.child.kill().await;
                Err(SpawnError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "helper stopped reading stdin",
                )))
            }
        }
    }

    /// Read the next helper event. `Ok(None)` means clean EOF.
    /// An overlong line never resumes mid-record: the reader drains
    /// to the next newline (bounded) and reports the oversize, so a
    /// corrupt stream restarts the helper instead of desyncing it.
    pub async fn next_event(&mut self) -> Result<Option<HelperEvent>, SpawnError> {
        self.stdout.next_event().await
    }

    pub async fn shutdown(mut self) {
        let _ = self.send(&HelperCommand::Shutdown).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
        let _ = self.child.kill().await;
    }
}

/// Return a log-safe command summary. Keep this as an allowlist: command
/// payloads can contain credentials, message content, filesystem paths,
/// phone numbers, and other private data.
pub fn redact_command(command: &HelperCommand) -> String {
    match command {
        HelperCommand::Hello => "hello".into(),
        HelperCommand::Login { .. } => "login".into(),
        HelperCommand::Logout { .. } => "logout".into(),
        HelperCommand::ListConversations { .. } => "list_conversations".into(),
        HelperCommand::FetchHistory { .. } => "fetch_history".into(),
        HelperCommand::SendText { request_id, .. } => {
            format!("send_text request_id={request_id}")
        }
        HelperCommand::SendMedia { request_id, .. } => {
            format!("send_media request_id={request_id}")
        }
        HelperCommand::React { request_id, .. } => format!("react request_id={request_id}"),
        HelperCommand::MarkRead { .. } => "mark_read".into(),
        HelperCommand::Typing { .. } => "typing".into(),
        HelperCommand::DeleteMessage { request_id, .. } => {
            format!("delete_message request_id={request_id}")
        }
        HelperCommand::OpenConversation { request_id, .. } => {
            format!("open_conversation request_id={request_id}")
        }
        HelperCommand::Sync { .. } => "sync".into(),
        HelperCommand::Shutdown => "shutdown".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_bounded() {
        assert_eq!(backoff_delay(0), Duration::from_secs(1));
        assert_eq!(backoff_delay(3), Duration::from_secs(8));
        assert_eq!(backoff_delay(6), Duration::from_secs(60));
        assert_eq!(backoff_delay(100), Duration::from_secs(60));
    }

    #[test]
    fn backoff_never_exceeds_one_minute() {
        for restarts in 0..200 {
            assert!(backoff_delay(restarts) <= Duration::from_secs(60));
        }
    }

    #[test]
    fn helper_env_override_missing_file_means_dormant() {
        let missing = "/tmp/handover-definitely-missing-helper-binary";
        assert_eq!(find_helper_in(Some(missing.as_ref()), None, None), None);
    }

    #[test]
    fn discovery_prefers_native_and_preserves_explicit_legacy_selection() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        let legacy = first.join(LEGACY_HELPER_BINARY);
        let native = second.join(HELPER_BINARY);
        std::fs::write(&legacy, []).unwrap();
        let paths = std::env::join_paths([&first, &second]).unwrap();
        assert_eq!(
            find_helper_in(None, None, Some(&paths)),
            Some(legacy.clone())
        );
        std::fs::write(&native, []).unwrap();
        assert_eq!(find_helper_in(None, None, Some(&paths)), Some(native));
        let bundled = first.join(HELPER_BINARY);
        std::fs::write(&bundled, []).unwrap();
        assert_eq!(
            find_helper_in(None, Some(&first), Some(&paths)),
            Some(bundled.clone())
        );
        assert_eq!(find_helper_in(None, Some(&first), None), Some(bundled));
        assert_eq!(
            find_helper_in(Some(legacy.as_os_str()), Some(&first), Some(&paths)),
            Some(legacy)
        );
        let missing = directory.path().join("missing");
        assert_eq!(
            find_helper_in(Some(missing.as_os_str()), Some(&first), Some(&paths)),
            None
        );
        assert_eq!(find_helper_in(None, None, None), None);
    }

    #[test]
    fn command_logs_only_include_allowlisted_summaries() {
        let redacted = redact_command(&HelperCommand::Login {
            account: "private-account".into(),
            bundle_b64: "private-bundle".into(),
        });
        assert_eq!(redacted, "login");

        let commands = [
            HelperCommand::SendText {
                request_id: "request".into(),
                account: "account".into(),
                conversation: "conversation".into(),
                text: "private-text".into(),
                reply_to: Some("private-reply-id".into()),
            },
            HelperCommand::SendMedia {
                request_id: "request".into(),
                account: "account".into(),
                conversation: "conversation".into(),
                path: "/private/path".into(),
                caption: Some("private-caption".into()),
            },
            HelperCommand::React {
                request_id: "request".into(),
                account: "account".into(),
                conversation: "conversation".into(),
                message: "private-message-id".into(),
                emoji: "private-emoji".into(),
                add: true,
            },
            HelperCommand::OpenConversation {
                request_id: "request".into(),
                account: "account".into(),
                addresses: vec!["private-phone-number".into()],
            },
        ];
        for command in commands {
            let summary = redact_command(&command);
            for private_value in [
                "private-text",
                "private-reply-id",
                "/private/path",
                "private-caption",
                "private-message-id",
                "private-emoji",
                "private-phone-number",
            ] {
                assert!(!summary.contains(private_value), "leaked {private_value}");
            }
            assert!(summary.contains("request_id=request"));
        }
    }

    #[tokio::test]
    async fn cancelled_partial_event_read_resumes_without_losing_bytes() {
        let (mut writer, reader) = tokio::io::duplex(1024);
        let mut events = EventReader::new(BufReader::new(reader));
        let event = br#"{"type":"hello","helper_protocol":1,"name":"test"}"#;
        writer.write_all(&event[..12]).await.expect("write prefix");

        assert!(
            tokio::time::timeout(Duration::from_millis(10), events.next_event())
                .await
                .is_err()
        );

        writer.write_all(&event[12..]).await.expect("write suffix");
        writer.write_all(b"\n").await.expect("write newline");
        assert_eq!(
            events.next_event().await.expect("event").expect("record"),
            HelperEvent::Hello {
                helper_protocol: 1,
                name: "test".into()
            }
        );
    }

    #[tokio::test]
    async fn unterminated_oversized_record_requires_helper_restart() {
        let prefix = vec![b'x'; 10 * 1024 * 1024 + 1];
        let mut events = EventReader::new(BufReader::new(prefix.as_slice()));
        assert!(
            matches!(events.next_event().await, Err(SpawnError::Io(error))
            if error.kind() == std::io::ErrorKind::InvalidData)
        );
        assert!(events.discarding);
    }

    #[tokio::test]
    async fn cancelled_oversized_discard_does_not_parse_its_remainder() {
        let (mut writer, reader) = tokio::io::duplex(MAX_HELPER_LINE_BYTES + 4096);
        let mut events = EventReader::new(BufReader::new(reader));
        let oversized_prefix = vec![b'x'; MAX_HELPER_LINE_BYTES + 8];
        writer
            .write_all(&oversized_prefix)
            .await
            .expect("write oversized prefix");

        assert!(
            tokio::time::timeout(Duration::from_millis(10), events.next_event())
                .await
                .is_err()
        );

        let valid = br#"{"type":"hello","helper_protocol":1,"name":"test"}"#;
        writer
            .write_all(b"\n")
            .await
            .expect("finish oversized line");
        writer.write_all(valid).await.expect("write valid event");
        writer.write_all(b"\n").await.expect("write event newline");

        assert!(matches!(
            events.next_event().await,
            Err(SpawnError::Handshake(
                crate::contract::ContractError::OversizedLine(_)
            ))
        ));
        assert_eq!(
            events
                .next_event()
                .await
                .expect("next event")
                .expect("record"),
            HelperEvent::Hello {
                helper_protocol: 1,
                name: "test".into()
            }
        );
    }
}
