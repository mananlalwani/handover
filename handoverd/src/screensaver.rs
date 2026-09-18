use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

static INHIBITOR: Mutex<Option<Child>> = Mutex::new(None);

/// Keep the desktop awake while a paired native phone is connected. The
/// external helper is used instead of assuming a particular desktop portal or
/// screensaver implementation. If it is unavailable, Handover leaves the
/// desktop policy unchanged.
pub(crate) fn update(native_connected: bool) {
    let mut inhibitor = INHIBITOR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(child) = inhibitor.as_mut() {
        if child.try_wait().ok().flatten().is_some() {
            *inhibitor = None;
        }
    }
    if native_connected && inhibitor.is_none() {
        let child = Command::new("systemd-inhibit")
            .args([
                "--what=idle:sleep",
                "--mode=block",
                "--who=Handover",
                "--why=Native phone connected",
                "sleep",
                "infinity",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(child) = child {
            *inhibitor = Some(child);
        }
    } else if !native_connected {
        if let Some(mut child) = inhibitor.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
