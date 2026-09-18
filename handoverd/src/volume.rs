use std::process::Stdio;

use handover_core::{VolumeAction, VolumeCommand};

pub(crate) fn execute(command: &VolumeCommand) {
    let args = match command.action {
        VolumeAction::Up => ["set-sink-volume", "@DEFAULT_SINK@", "5%+"],
        VolumeAction::Down => ["set-sink-volume", "@DEFAULT_SINK@", "5%-"],
        VolumeAction::ToggleMute => ["set-sink-mute", "@DEFAULT_SINK@", "toggle"],
    };
    let _ = std::process::Command::new("pactl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
