use std::process::{Command, Stdio};

use handover_core::{RemoteInputAction, RemoteInputCommand};

pub(crate) fn execute(command: &RemoteInputCommand) {
    if matches!(command.action, RemoteInputAction::Type)
        && std::env::var_os("WAYLAND_DISPLAY").is_some()
        && command_available("wtype")
    {
        let _ = crate::local_cmd::status_timeout(
            Command::new("wtype")
                .arg("--")
                .arg(command.text.as_deref().unwrap_or_default()),
        );
        return;
    }
    let mut process = Command::new("xdotool");
    process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.action {
        RemoteInputAction::Move => {
            process.args(["mousemove_relative", "--"]);
            process.arg(command.delta_x.to_string());
            process.arg(command.delta_y.to_string());
        }
        RemoteInputAction::Click => {
            process.args(["click", &command.button.to_string()]);
        }
        RemoteInputAction::Scroll => {
            let button = if command.delta_y < 0 { "4" } else { "5" };
            process.args([
                "click",
                "--repeat",
                &command.delta_y.unsigned_abs().min(20).to_string(),
                button,
            ]);
        }
        RemoteInputAction::Type => {
            process.args(["type", "--delay", "0"]);
            process.arg(command.text.as_deref().unwrap_or_default());
        }
    }
    let _ = crate::local_cmd::status_timeout(&mut process);
}

fn command_available(command: &str) -> bool {
    crate::local_cmd::status_timeout(Command::new("sh").args([
        "-c",
        "command -v -- \"$1\" >/dev/null 2>&1",
        "handover",
        command,
    ]))
    .is_ok_and(|status| status.success())
}
