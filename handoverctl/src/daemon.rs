use clap::Command;

/// Synthetic CLI definition for the daemon.
///
/// `handoverd` accepts no arguments. This definition exists so the
/// section 8 man page renders from code through `clap_mangen`, next to
/// the `handoverctl(1)` page. Keep the summary aligned with
/// `docs/cli/operations.md`.
pub fn command() -> Command {
    Command::new("handoverd")
        .disable_help_flag(true)
        .about("Handover Android device integration daemon")
        .long_about(
            "Handover Android device integration daemon.\n\n\
            handoverd owns authoritative runtime state for paired Android devices: \
            presence, notifications, media sessions, clipboard, calls, shares, and messaging. \
            It listens on $XDG_RUNTIME_DIR/handover/handoverd.sock. \
            handoverctl and the Quickshell client connect, read a snapshot, and rebuild their views.\n\n\
            handoverd accepts no arguments. It shuts down gracefully on SIGTERM or SIGINT, \
            stopping the messaging helper and releasing the desktop idle inhibitor.",
        )
        .after_help(
            "Environment: HANDOVER_GMESSAGES_HELPER, HANDOVER_GMESSAGES_STAGING_DIR, \
            HANDOVER_CLIPBOARD_MIRROR. Debug builds also honor HANDOVER_NATIVE_SMOKE_PORT.\n\
            See handoverctl(1) and docs/cli/operations.md for sockets, state paths, and troubleshooting.",
        )
}
