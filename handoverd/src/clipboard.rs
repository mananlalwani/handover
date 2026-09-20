use std::fs::File;
use std::io;
use std::process::Command;

use handover_core::ClipboardText;

pub(crate) fn apply(text: &ClipboardText) {
    let (value, mime) = if let Some(uri) = &text.uri {
        (uri.as_str(), "text/uri-list")
    } else if let Some(html) = &text.html {
        (html.as_str(), "text/html")
    } else {
        (text.text.as_str(), "text/plain")
    };
    let _ = write_clipboard(value, mime);
}

pub(crate) fn apply_plain(text: &str) -> io::Result<()> {
    write_clipboard(text, "text/plain")
}

fn write_clipboard(value: &str, mime: &str) -> io::Result<()> {
    let status = crate::local_cmd::feed_and_wait(
        Command::new("wl-copy").args(["--type", mime]),
        value.as_bytes().to_vec(),
    )?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("wl-copy failed"))
    }
}

pub(crate) fn apply_file(path: &str, mime: &str) {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return,
    };
    let _ = crate::local_cmd::feed_file(Command::new("wl-copy").args(["--type", mime]), file);
}

/// MIME types currently offered by the Wayland clipboard, or `None` when the
/// clipboard is unreadable.
pub(crate) async fn offered_types() -> Option<Vec<String>> {
    let output = read_bounded(&["--list-types"], 64 * 1024).await?;
    Some(
        String::from_utf8_lossy(&output)
            .lines()
            .map(|line| line.trim().to_owned())
            .filter(|line| !line.is_empty())
            .collect(),
    )
}

/// Read at most 32 KiB of text for one offered MIME type.
pub(crate) async fn read_text_mime(mime: &str) -> Option<String> {
    let mime = mime.to_owned();
    let output = read_bounded(&["--no-newline", "--type", &mime], 32 * 1024).await?;
    if output.len() > 32 * 1024 {
        return None;
    }
    String::from_utf8(output).ok()
}

/// Run `wl-paste` off the async runtime: bounded output, bounded time.
async fn read_bounded(args: &[&str], limit: usize) -> Option<Vec<u8>> {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    tokio::task::spawn_blocking(move || {
        let mut command = std::process::Command::new("wl-paste");
        command.args(&args);
        let output = crate::local_cmd::output_bounded(&mut command, limit).ok()?;
        output.status.success().then_some(output.stdout)
    })
    .await
    .ok()?
}
