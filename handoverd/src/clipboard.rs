use std::io::Write;
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
