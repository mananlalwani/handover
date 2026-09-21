use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::limits::*;
use crate::protocol::*;

pub(crate) fn clipboard_temp_path(message: &Message) -> Option<&PathBuf> {
    match message {
        Message::ShareFile {
            clipboard: true,
            path,
            ..
        } => Some(path),
        _ => None,
    }
}

/// Delete a queued clipboard share's temp file. Every terminal path
/// for a queued entry (streamed, cancelled, expired, timed out,
/// disconnected, send-failed) must call this.
pub(crate) fn delete_clipboard_temp(message: &Message) {
    if let Some(path) = clipboard_temp_path(message) {
        let _ = fs::remove_file(path);
    }
}

pub(crate) fn list_local_directory(path: &str) -> Result<Vec<WireFileEntry>, String> {
    let root = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "unavailable".to_owned())?;
    list_directory_under(root, path)
}

pub(crate) fn list_directory_under(
    root: PathBuf,
    path: &str,
) -> Result<Vec<WireFileEntry>, String> {
    if !safe_browse_path(path) {
        return Err("unavailable".to_owned());
    }
    let root = fs::canonicalize(&root).map_err(|_| "unavailable".to_owned())?;
    let requested = if path == "." {
        root.clone()
    } else {
        root.join(path)
    };
    let directory = fs::canonicalize(&requested).map_err(|_| "unavailable".to_owned())?;
    if !directory.starts_with(&root) {
        return Err("unavailable".to_owned());
    }
    if !fs::metadata(&directory)
        .map_err(|_| "unavailable".to_owned())?
        .is_dir()
    {
        return Err("unavailable".to_owned());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(directory).map_err(|_| "unavailable".to_owned())? {
        let entry = entry.map_err(|_| "unavailable".to_owned())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !safe_share_name(&name) {
            continue;
        }
        let metadata = entry.metadata().ok();
        entries.push(WireFileEntry {
            name,
            directory: metadata.as_ref().is_some_and(std::fs::Metadata::is_dir),
            size: metadata
                .filter(std::fs::Metadata::is_file)
                .map(|value| value.len()),
        });
        if entries.len() >= 512 {
            break;
        }
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(entries)
}

pub(crate) fn safe_command_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}

pub(crate) fn native_custom_commands() -> Vec<(String, Vec<String>)> {
    let Some(base) = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    else {
        return Vec::new();
    };
    let Ok(contents) = fs::read_to_string(base.join("handover/custom-commands.toml")) else {
        return Vec::new();
    };
    parse_custom_commands(&contents)
}

pub(crate) fn parse_custom_commands(text: &str) -> Vec<(String, Vec<String>)> {
    let mut commands = Vec::new();
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
                    commands.push((name, argv));
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
                        commands.push((name, argv));
                    }
                }
                let name = unquote_toml(value.trim());
                if safe_command_name(&name) {
                    current = Some((name, Vec::new()));
                }
            }
            "argv" => {
                if let Some((_, argv)) = current.as_mut() {
                    *argv = parse_toml_string_list(value.trim());
                }
            }
            _ => {}
        }
    }
    if let Some((name, argv)) = current.take() {
        if !argv.is_empty() {
            commands.push((name, argv));
        }
    }
    commands
}

fn unquote_toml(value: &str) -> String {
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

fn parse_toml_string_list(value: &str) -> Vec<String> {
    let value = value.trim();
    if !value.starts_with('[') || !value.ends_with(']') {
        return Vec::new();
    }
    let inner = &value[1..value.len() - 1];
    let mut items = Vec::new();
    for part in inner.split(',') {
        let item = unquote_toml(part);
        if item.is_empty() || item.len() > 1024 || item.contains('\0') {
            return Vec::new();
        }
        items.push(item);
    }
    items
}

pub(crate) fn run_native_custom_command(name: &str) -> (bool, Option<i32>, Option<String>) {
    let Some((_, argv)) = native_custom_commands()
        .into_iter()
        .find(|(candidate, _)| candidate == name)
    else {
        return (false, None, Some("unknown_command".into()));
    };
    let Some((program, args)) = argv.split_first() else {
        return (false, None, Some("spawn_failed".into()));
    };
    let mut child = match std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return (false, None, Some("spawn_failed".into())),
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (true, status.code(), None),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return (false, None, Some("timed_out".into()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return (false, None, Some("spawn_failed".into())),
        }
    }
}

pub(crate) fn new_transfer_id() -> Result<String, openssl::error::ErrorStack> {
    let mut bytes = [0u8; 16];
    openssl::rand::rand_bytes(&mut bytes)?;
    Ok(hex::encode(bytes))
}

pub(crate) fn take_expired_shares(
    pending: &mut BTreeMap<String, Instant>,
    now: Instant,
) -> Vec<String> {
    let expired = pending
        .iter()
        .filter(|(_, started)| now.duration_since(**started) >= SHARE_RESULT_TIMEOUT)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for id in &expired {
        pending.remove(id);
    }
    expired
}

pub(crate) fn stream_file_until<R: Read, W: Write>(
    input: &mut R,
    output: &mut W,
    size: u64,
    started: Instant,
    progress: &mut impl FnMut(u64),
) -> std::io::Result<()> {
    let mut remaining = size;
    let mut next_progress = 256 * 1024;
    let mut buffer = [0u8; SHARE_BUFFER];
    progress(0);
    while remaining > 0 {
        if started.elapsed() >= SHARE_RESULT_TIMEOUT {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "share result deadline",
            ));
        }
        let amount = usize::try_from(remaining.min(SHARE_BUFFER as u64)).unwrap();
        input.read_exact(&mut buffer[..amount])?;
        output.write_all(&buffer[..amount])?;
        remaining -= amount as u64;
        let sent = size - remaining;
        if sent >= next_progress || remaining == 0 {
            progress(sent);
            next_progress = sent.saturating_add(256 * 1024);
        }
    }
    output.flush()
}

pub(crate) fn read_exact_until<R: Read>(
    input: &mut R,
    buffer: &mut [u8],
    deadline: Instant,
) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < buffer.len() {
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "inbound transfer deadline",
            ));
        }
        match input.read(&mut buffer[offset..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "inbound transfer ended early",
                ));
            }
            Ok(amount) => offset += amount,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "inbound transfer deadline",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
