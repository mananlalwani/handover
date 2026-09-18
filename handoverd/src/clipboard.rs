use std::io::Write;
use std::process::{Command, Stdio};

use handover_core::ClipboardText;

pub(crate) fn apply(text: &ClipboardText) {
    let mut child = match Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return,
    };
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(text.text.as_bytes());
    }
    let _ = child.wait();
}
