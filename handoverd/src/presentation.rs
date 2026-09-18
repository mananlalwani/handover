use std::process::Stdio;

use handover_core::{PresentationAction, PresentationCommand};

/// Execute presentation controls through the user's existing X11 input
/// bridge. Wayland compositors that do not expose XWayland input will reject
/// these commands rather than receiving unverified synthetic input.
pub(crate) fn execute(command: &PresentationCommand) {
    let mut process = std::process::Command::new("xdotool");
    match command.action {
        PresentationAction::Previous => process.args(["key", "Left"]),
        PresentationAction::Next => process.args(["key", "Right"]),
        PresentationAction::Start => process.args(["key", "F5"]),
        PresentationAction::Stop => process.args(["key", "Escape"]),
        PresentationAction::Fullscreen => process.args(["key", "F11"]),
        PresentationAction::PointerMove => process.args([
            "mousemove_relative",
            &command.delta_x.to_string(),
            &command.delta_y.to_string(),
        ]),
        PresentationAction::PointerClick => process.args(["click", "1"]),
    };
    let _ = process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
