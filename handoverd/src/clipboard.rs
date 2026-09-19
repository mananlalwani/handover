use std::fs::File;
use std::io::Write;
use std::io::copy;
use std::process::{Command, Stdio};

use handover_core::ClipboardText;

pub(crate) fn apply(text: &ClipboardText) {
    let (value, mime) = if let Some(uri) = &text.uri {
        (uri.as_str(), "text/uri-list")
    } else if let Some(html) = &text.html {
        (html.as_str(), "text/html")
    } else {
        (text.text.as_str(), "text/plain")
    };
    let mut child = match Command::new("wl-copy")
        .args(["--type", mime])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return,
    };
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(value.as_bytes());
    }
    let _ = child.wait();
}

pub(crate) fn apply_file(path: &str, mime: &str) {
    let mut child = match Command::new("wl-copy")
        .args(["--type", mime])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return,
    };
    if let (Ok(mut input), Some(stdin)) = (File::open(path), child.stdin.as_mut()) {
        let _ = copy(&mut input, stdin);
    }
    let _ = child.wait();
}

/// MIME types currently offered by the Wayland clipboard, or `None` when the
/// clipboard is unreadable.
pub(crate) async fn offered_types() -> Option<Vec<String>> {
    let output = tokio::process::Command::new("wl-paste")
        .arg("--list-types")
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| line.trim().to_owned())
            .filter(|line| !line.is_empty())
            .collect(),
    )
}

/// Read at most 32 KiB of text for one offered MIME type.
pub(crate) async fn read_text_mime(mime: &str) -> Option<String> {
    let output = tokio::process::Command::new("wl-paste")
        .args(["--no-newline", "--type", mime])
        .output()
        .await
        .ok()?;
    if !output.status.success() || output.stdout.len() > 32 * 1024 {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}
