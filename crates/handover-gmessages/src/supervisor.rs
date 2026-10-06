//! Daemon-side helper supervision.
//!
//! The helper is optional: when no helper binary is configured or found,
//! messaging is simply unavailable and every other subsystem keeps working.
//! A crashing or misbehaving helper marks its accounts disconnected; it can
//! never disturb native battery/notifications/media/sharing or KDE Connect.
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

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
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
    stdout: BufReader<ChildStdout>,
    pub name: String,
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
            stdout: BufReader::new(stdout),
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
        const MAX_DISCARD_BYTES: usize = 10 * 1024 * 1024;
        let mut line = Vec::new();
        let mut discarding = false;
        let mut discarded = 0;
        loop {
            let available = self.stdout.fill_buf().await.map_err(SpawnError::Io)?;
            if available.is_empty() {
                if discarding || line.is_empty() {
                    return if discarding {
                        Err(SpawnError::Handshake(
                            crate::contract::ContractError::OversizedLine(line.len() + discarded),
                        ))
                    } else {
                        Ok(None)
                    };
                }
                break;
            }

            let newline = available.iter().position(|byte| *byte == b'\n');
            let consumed = newline.map_or(available.len(), |position| position + 1);
            let content_length = newline.unwrap_or(available.len());
            if discarding {
                discarded += content_length;
                if discarded > MAX_DISCARD_BYTES {
                    return Err(SpawnError::Handshake(
                        crate::contract::ContractError::OversizedLine(discarded),
                    ));
                }
            } else if line.len() + content_length > MAX_HELPER_LINE_BYTES {
                discarding = true;
                discarded = line.len() + content_length;
                line.clear();
                if discarded > MAX_DISCARD_BYTES {
                    return Err(SpawnError::Handshake(
                        crate::contract::ContractError::OversizedLine(discarded),
                    ));
                }
            } else {
                line.extend_from_slice(&available[..content_length]);
            }
            self.stdout.consume(consumed);
            if newline.is_some() {
                if discarding {
                    return Err(SpawnError::Handshake(
                        crate::contract::ContractError::OversizedLine(discarded),
                    ));
                }
                break;
            }
        }
        decode_event(&line).map(Some).map_err(SpawnError::Handshake)
    }

    pub async fn shutdown(mut self) {
        let _ = self.send(&HelperCommand::Shutdown).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
        let _ = self.child.kill().await;
    }
}

/// Redact a command for logging: `Login` bundles never appear in logs.
pub fn redact_command(command: &HelperCommand) -> String {
    match serde_json::to_value(command) {
        Ok(mut value) => {
            if let Some(object) = value.as_object_mut() {
                object.remove("bundle_b64");
            }
            value.to_string()
        }
        Err(_) => "unserializable command".to_string(),
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
    fn login_command_is_redacted_for_logs() {
        let redacted = redact_command(&HelperCommand::Login {
            account: "work".into(),
            bundle_b64: "c2VjcmV0LWJ5dGVz".into(),
        });
        assert!(!redacted.contains("c2VjcmV0LWJ5dGVz"));
        assert!(redacted.contains("work"));
    }
}
