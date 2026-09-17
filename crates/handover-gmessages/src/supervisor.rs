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

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::contract::{
    HelperCommand, HelperEvent, MAX_HELPER_LINE_BYTES, check_hello, decode_event, encode_command,
};

/// Environment override for the helper binary path. Never carries secrets.
pub const HELPER_ENV: &str = "HANDOVER_GMESSAGES_HELPER";
/// Default helper binary name resolved via `PATH`.
pub const HELPER_BINARY: &str = "handover-gmessages-helper";

/// Locate the helper binary: explicit env path first, then `PATH` lookup.
/// Returns `None` when messaging should stay dormant.
pub fn find_helper() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(HELPER_ENV) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
        return None;
    }
    // PATH lookup without spawning anything: messaging stays dormant when
    // no helper binary is installed.
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join(HELPER_BINARY))
            .find(|candidate| candidate.is_file())
    })
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
        process.send(&HelperCommand::Hello).await?;
        let event = process.next_event().await?.ok_or(SpawnError::NoOutput)?;
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
        self.stdin
            .write_all(encoded.as_bytes())
            .await
            .map_err(SpawnError::Io)?;
        self.stdin.write_all(b"\n").await.map_err(SpawnError::Io)?;
        self.stdin.flush().await.map_err(SpawnError::Io)?;
        Ok(())
    }

    /// Read the next helper event. `Ok(None)` means clean EOF.
    pub async fn next_event(&mut self) -> Result<Option<HelperEvent>, SpawnError> {
        let mut line = Vec::new();
        loop {
            let available = self.stdout.fill_buf().await.map_err(SpawnError::Io)?;
            if available.is_empty() {
                if line.is_empty() {
                    return Ok(None);
                }
                break;
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let consumed = newline.map_or(available.len(), |position| position + 1);
            let content_length = newline.unwrap_or(available.len());
            if line.len() + content_length > MAX_HELPER_LINE_BYTES {
                return Err(SpawnError::Handshake(
                    crate::contract::ContractError::OversizedLine(line.len() + content_length),
                ));
            }
            line.extend_from_slice(&available[..content_length]);
            self.stdout.consume(consumed);
            if newline.is_some() {
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
        // Test-only env mutation: single-threaded test context.
        unsafe {
            std::env::set_var(HELPER_ENV, missing);
        }
        assert_eq!(find_helper(), None);
        unsafe {
            std::env::remove_var(HELPER_ENV);
        }
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
