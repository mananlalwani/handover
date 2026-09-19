use std::process::Stdio;

use handover_core::{VolumeAction, VolumeCommand};
use tracing::warn;

pub(crate) fn execute(command: &VolumeCommand) {
    if run_wpctl(command.action) || run_pactl(command.action) {
        return;
    }
    warn!(action = ?command.action, "no desktop volume backend accepted the command");
}

fn run_wpctl(action: VolumeAction) -> bool {
    let mut process = std::process::Command::new("wpctl");
    match action {
        VolumeAction::Up => process.args([
            "set-volume",
            "--limit",
            "1.0",
            "@DEFAULT_AUDIO_SINK@",
            "5%+",
        ]),
        VolumeAction::Down => process.args(["set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"]),
        VolumeAction::ToggleMute => process.args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]),
    };
    succeeded(&mut process)
}

fn run_pactl(action: VolumeAction) -> bool {
    let mut process = std::process::Command::new("pactl");
    match action {
        VolumeAction::Up | VolumeAction::Down => {
            let Some(current) = pactl_volume_percent() else {
                return false;
            };
            let target = match action {
                VolumeAction::Up => current.saturating_add(5).min(100),
                VolumeAction::Down => current.saturating_sub(5),
                VolumeAction::ToggleMute => unreachable!(),
            };
            process.args(["set-sink-volume", "@DEFAULT_SINK@", &format!("{target}%")])
        }
        VolumeAction::ToggleMute => process.args(["set-sink-mute", "@DEFAULT_SINK@", "toggle"]),
    };
    succeeded(&mut process)
}

fn pactl_volume_percent() -> Option<u8> {
    let output = std::process::Command::new("pactl")
        .args(["get-sink-volume", "@DEFAULT_SINK@"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .split_whitespace()
        .find_map(|field| field.strip_suffix('%')?.parse().ok())
}

fn succeeded(process: &mut std::process::Command) -> bool {
    process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}
