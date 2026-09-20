//! Allowlisted desktop commands.
//!
//! The user defines fixed commands in
//! `$XDG_CONFIG_HOME/handover/custom-commands.toml` (or
//! `~/.config/handover/custom-commands.toml`). Only listed commands run, with
//! fixed arguments and no shell, interpolation, or caller-supplied input. A
//! missing or unreadable file means the feature is dormant, never an error
//! for other daemon paths.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use handover_core::{CustomCommandEntry, CustomCommandFailure, CustomCommandResult};
use tokio::process::Command;

const RUN_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_NAME_LEN: usize = 64;
/// Upper bound for captured stdout/stderr of one allowlisted command.
const MAX_OUTPUT_BYTES: u64 = 1024 * 1024;

/// Run one allowlisted command with a wall-time timeout, a kill on
/// expiry, and a cap on captured output. Output content never leaves
/// this function (only the exit status is reported), so overlong
/// streams are truncated, not failed.
async fn run_capped(program: &str, args: &[String]) -> Result<std::process::Output, RunError> {
    use tokio::io::AsyncReadExt;
    let mut child = Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| RunError::Spawn)?;
    let stdout = child.stdout.take().ok_or(RunError::Spawn)?;
    let stderr = child.stderr.take().ok_or(RunError::Spawn)?;
    let (out, err, status) = tokio::time::timeout(RUN_TIMEOUT, async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut out_take = stdout.take(MAX_OUTPUT_BYTES + 1);
        let mut err_take = stderr.take(MAX_OUTPUT_BYTES + 1);
        let (out_read, err_read, status) = tokio::join!(
            out_take.read_to_end(&mut out),
            err_take.read_to_end(&mut err),
            child.wait()
        );
        out_read.map_err(|_| RunError::Spawn)?;
        err_read.map_err(|_| RunError::Spawn)?;
        let status = status.map_err(|_| RunError::Spawn)?;
        Ok::<_, RunError>((out, err, status))
    })
    .await
    .map_err(|_| RunError::Timeout)??;
    let mut out = out;
    let mut err = err;
    out.truncate(MAX_OUTPUT_BYTES as usize);
    err.truncate(MAX_OUTPUT_BYTES as usize);
    Ok(std::process::Output {
        status,
        stdout: out,
        stderr: err,
    })
}

#[derive(Debug)]
enum RunError {
    Spawn,
    Timeout,
}

pub(crate) fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    base.map(|base| base.join("handover").join("custom-commands.toml"))
        .unwrap_or_else(|| PathBuf::from("/dev/null"))
}

/// Load and validate the allowlist. Unknown tables and keys are ignored so a
/// newer file never breaks an older daemon; invalid entries are skipped.
pub(crate) fn load() -> BTreeMap<String, Vec<String>> {
    let path = config_path();
    let text = std::fs::read_to_string(path).unwrap_or_default();
    parse(&text)
}

fn parse(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut commands = BTreeMap::new();
    let mut current: Option<(String, Vec<String>)> = None;
    let mut in_command = false;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            if let Some((name, argv)) = current.take() {
                if !argv.is_empty() {
                    commands.insert(name, argv);
                }
            }
            in_command = line == "[[command]]";
            continue;
        }
        if !in_command {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "name" => {
                if let Some((name, argv)) = current.take() {
                    if !argv.is_empty() {
                        commands.insert(name, argv);
                    }
                }
                let name = unquote(value.trim());
                if valid_name(&name) {
                    current = Some((name, Vec::new()));
                }
            }
            "argv" => {
                if let Some((_, argv)) = current.as_mut() {
                    *argv = parse_string_list(value.trim());
                }
            }
            _ => {}
        }
    }
    if let Some((name, argv)) = current.take() {
        if !argv.is_empty() {
            commands.insert(name, argv);
        }
    }
    commands
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2
        && value.starts_with('"')
        && value.ends_with('"')
        && !value[1..value.len() - 1].contains('"')
    {
        value[1..value.len() - 1].to_owned()
    } else {
        String::new()
    }
}

fn parse_string_list(value: &str) -> Vec<String> {
    let value = value.trim();
    if !value.starts_with('[') || !value.ends_with(']') {
        return Vec::new();
    }
    let inner = &value[1..value.len() - 1];
    let mut items = Vec::new();
    for part in inner.split(',') {
        let item = unquote(part);
        if item.is_empty() || item.len() > 1024 || item.contains('\0') {
            return Vec::new();
        }
        items.push(item);
    }
    items
}

pub(crate) fn entries() -> Vec<CustomCommandEntry> {
    load()
        .into_iter()
        .map(|(name, argv)| CustomCommandEntry { name, argv })
        .collect()
}

/// Run one allowlisted command. No shell is involved and the caller supplies
/// nothing beyond the name; unknown names are rejected before any spawn.
pub(crate) async fn run(name: &str) -> CustomCommandResult {
    run_with(name, &load()).await
}

fn run_with(
    name: &str,
    commands: &BTreeMap<String, Vec<String>>,
) -> impl Future<Output = CustomCommandResult> {
    let result = commands.get(name).cloned();
    async move {
        let Some(argv) = result else {
            return CustomCommandResult {
                name: name.to_owned(),
                accepted: false,
                exit_code: None,
                failure: Some(CustomCommandFailure::UnknownCommand),
            };
        };
        let (program, args) = argv.split_first().expect("validated non-empty");
        // Capped stdout: a chatty allowlisted program must not balloon
        // the daemon. The timeout still bounds wall time.
        let output = run_capped(program, args).await;
        match output {
            Err(RunError::Timeout) => CustomCommandResult {
                name: name.to_owned(),
                accepted: false,
                exit_code: None,
                failure: Some(CustomCommandFailure::TimedOut),
            },
            Err(RunError::Spawn) => CustomCommandResult {
                name: name.to_owned(),
                accepted: false,
                exit_code: None,
                failure: Some(CustomCommandFailure::SpawnFailed),
            },
            Ok(output) => CustomCommandResult {
                name: name.to_owned(),
                accepted: true,
                exit_code: output.status.code(),
                failure: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_allowlist_parses_name_and_argv() {
        let commands = parse(
            "# stay-awake helpers\n[[command]]\nname = \"lock-screen\"\nargv = [\"loginctl\", \"lock-session\"]\n",
        );
        assert_eq!(
            commands.get("lock-screen"),
            Some(&vec!["loginctl".to_owned(), "lock-session".to_owned()])
        );
    }

    #[test]
    fn invalid_entries_are_skipped_without_breaking_valid_ones() {
        let commands = parse(
            "[[command]]\nname = \"Bad Name!\"\nargv = [\"x\"]\n\
             [[command]]\nname = \"good\"\nargv = \"not-a-list\"\n\
             [[command]]\nname = \"ok\"\nargv = [\"true\"]\n",
        );
        assert_eq!(commands.len(), 1);
        assert!(commands.contains_key("ok"));
    }

    #[tokio::test]
    async fn unknown_command_is_rejected_before_spawn() {
        let commands = BTreeMap::new();
        let result = run_with("no-such-command", &commands).await;
        assert!(!result.accepted);
        assert_eq!(result.failure, Some(CustomCommandFailure::UnknownCommand));
        assert_eq!(result.exit_code, None);
    }

    #[tokio::test]
    async fn allowlisted_program_runs_without_shell() {
        let commands = BTreeMap::from([("succeed".to_owned(), vec!["true".to_owned()])]);
        let result = run_with("succeed", &commands).await;
        assert!(result.accepted);
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.failure, None);
    }

    #[tokio::test]
    async fn missing_program_reports_spawn_failure() {
        let commands = BTreeMap::from([(
            "missing".to_owned(),
            vec!["handover-no-such-binary".to_owned()],
        )]);
        let result = run_with("missing", &commands).await;
        assert!(!result.accepted);
        assert_eq!(result.failure, Some(CustomCommandFailure::SpawnFailed));
    }
}
