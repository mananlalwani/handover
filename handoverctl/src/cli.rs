use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Inspect and control Handover devices through the local daemon.
///
/// Every command that talks to a device first connects to handoverd over
/// the Unix socket at `$XDG_RUNTIME_DIR/handover/handoverd.sock` and reads
/// a current snapshot. A message that says a request was accepted means the
/// daemon or backend took the request. It does not mean the phone acted on
/// it. Watch `handoverctl monitor` for the later result.
#[derive(Debug, Parser)]
#[command(
    name = "handoverctl",
    about = "Inspect and control Handover",
    long_about = "Inspect and control Handover devices through the local handoverd daemon.\n\nCommands that change remote state report acceptance, not completion. \
        The daemon owns authoritative state; this CLI rebuilds its view from a fresh snapshot on each run. \
        handoverd must be running and reachable over $XDG_RUNTIME_DIR/handover/handoverd.sock.",
    version,
    after_help = "Exit status: 0 on success, 1 on any error (daemon unreachable, unknown device, rejected request).\nRun `handoverctl <command> --help` for per-command detail."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List devices known to handoverd.
    ///
    /// Prints the current daemon snapshot: name, connection state, pairing
    /// state, and battery. Requires a running handoverd.
    Devices,
    /// Inspect and manage native Android pairing and device commands.
    ///
    /// Peer selection accepts a peer ID or, when unique, a device name.
    /// Ping, ring, lock, keep-awake, and tethering queue a request and
    /// report acceptance only. Placing a real phone call requires --confirm.
    Native {
        #[command(subcommand)]
        command: NativeCommand,
    },
    /// List active remote notifications from the daemon snapshot.
    Notifications,
    /// List or request an on-demand native contacts snapshot.
    Contacts {
        #[command(subcommand)]
        command: ContactsCommand,
    },
    /// List media sessions, or send one playback command.
    ///
    /// With no subcommand, prints the current media snapshot. With a
    /// subcommand, the command is accepted by the daemon; playback change
    /// is not confirmed. Sessions are selected by `DEVICE:PLAYER` or, when
    /// unique, by application name.
    Media {
        #[command(subcommand)]
        command: Option<MediaSubcommand>,
    },
    /// Print live normalized device and notification changes.
    ///
    /// Subscribes to daemon events, prints the initial counts, then prints
    /// each event until interrupted with Ctrl-C. Reconnects with a short
    /// delay when the daemon is unavailable.
    Monitor,
    /// Send a URL to one paired device.
    ///
    /// The share is accepted and tracked when the backend supports it. A
    /// printed transfer id means the request was accepted, not delivered.
    /// Watch `monitor` for the receiver result.
    SendUrl {
        /// Device name (when unique) or device ID.
        #[arg(value_name = "DEVICE")]
        device: String,
        /// URL to send.
        #[arg(value_name = "URL")]
        url: String,
    },
    /// Send a notification to one native phone.
    ///
    /// Queued on the native backend. Delivery is not confirmed by this
    /// command. Use `monitor` to observe the Android result.
    Notify {
        /// Device name (when unique) or device ID.
        #[arg(value_name = "DEVICE")]
        device: String,
        /// Sending application label shown on the phone.
        #[arg(value_name = "APP")]
        app: String,
        /// Notification title.
        #[arg(value_name = "TITLE")]
        title: String,
        /// Notification body text.
        #[arg(value_name = "BODY")]
        body: String,
    },
    /// Set the clipboard on one native phone.
    ///
    /// With TEXT, sends that text. Without TEXT, reads the current Wayland
    /// clipboard and sends it. Native clipboard text is limited to 32 KiB
    /// and is not stored in daemon state.
    Clipboard {
        /// Device name (when unique) or device ID.
        #[arg(value_name = "DEVICE")]
        device: String,
        /// Text to send. If omitted, read the current Wayland clipboard.
        #[arg(value_name = "TEXT")]
        text: Option<String>,
    },
    /// Send one local file to a paired device.
    ///
    /// The path is canonicalized locally and shared as a file URL. The
    /// share is accepted, not delivered. Watch `monitor` for the result.
    SendFile {
        /// Device name (when unique) or device ID.
        #[arg(value_name = "DEVICE")]
        device: String,
        /// Local file to send.
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
    /// Control the desktop idle inhibitor: inhibit, release, or follow.
    ///
    /// `inhibit` forces the inhibitor on, `release` forces it off, `follow`
    /// returns to the automatic policy. Prints the resulting override.
    Screensaver {
        /// One of: inhibit, release, follow.
        #[arg(value_name = "ACTION")]
        action: String,
    },
    /// Get or set Linux-to-phone background clipboard mirroring (off by default).
    ///
    /// `on` enables text-only background mirroring, `off` disables it,
    /// `status` prints the current state.
    ClipboardMirror {
        /// One of: on, off, status.
        #[arg(value_name = "ACTION")]
        action: String,
    },
    /// List, pin, copy, and clear phone-to-Linux clipboard history.
    ClipboardHistory {
        #[command(subcommand)]
        command: ClipboardHistoryCommand,
    },
    /// Cancel a queued native share that has not started streaming.
    ///
    /// Reports whether the share was cancelled or was already sent or
    /// unknown. Cancellation only works before streaming starts.
    CancelShare {
        /// Device name (when unique) or device ID.
        #[arg(value_name = "DEVICE")]
        device: String,
        /// Transfer id printed by the accepting send command.
        #[arg(value_name = "TRANSFER_ID")]
        transfer_id: String,
    },
    /// List or run allowlisted desktop commands.
    ///
    /// Only commands in the daemon allowlist can run. Each runs with fixed
    /// arguments, no shell, and no caller-supplied input.
    Custom {
        #[command(subcommand)]
        command: CustomCommand,
    },
    /// Print current normalized call state for one device.
    ///
    /// Reads the daemon snapshot. Prints the call phase and available
    /// controls, or reports that no call state is attested.
    Calls {
        /// Device name (when unique) or device ID.
        #[arg(value_name = "DEVICE")]
        device: String,
    },
    /// Inspect and use messaging accounts and conversations.
    ///
    /// Sends are accepted with a request id; delivery, display, and failure
    /// arrive later as attested status events. Credential bundles for
    /// `login` are read from a file or stdin, never from argv.
    Messages {
        #[command(subcommand)]
        command: MessagesCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum CustomCommand {
    /// List allowlisted desktop commands with their fixed argv.
    List,
    /// Run one allowlisted desktop command by name.
    Run {
        /// Allowlisted command name, as shown by `custom list`.
        #[arg(value_name = "NAME")]
        name: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ClipboardHistoryCommand {
    /// List recent and pinned entries (text previews only).
    List,
    /// Save a new pinned string.
    Save {
        /// Text to pin.
        #[arg(value_name = "TEXT")]
        text: String,
    },
    /// Pin an existing entry by id.
    Pin {
        /// History entry id from `clipboard-history list`.
        #[arg(value_name = "ID")]
        id: u64,
    },
    /// Unpin an existing entry by id.
    Unpin {
        /// History entry id from `clipboard-history list`.
        #[arg(value_name = "ID")]
        id: u64,
    },
    /// Copy an entry into the Linux clipboard.
    Copy {
        /// History entry id from `clipboard-history list`.
        #[arg(value_name = "ID")]
        id: u64,
    },
    /// Remove recent entries, retaining pins unless --all is passed.
    Clear {
        /// Also remove pinned entries.
        #[arg(long)]
        all: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ContactsCommand {
    /// List the latest contacts snapshot held by handoverd.
    List,
    /// Ask one native phone to send a fresh contacts snapshot.
    ///
    /// Requests a snapshot; arrival is not confirmed by this command.
    Sync {
        /// Device name (when unique) or device ID.
        #[arg(value_name = "DEVICE")]
        device: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum NativeCommand {
    /// List trusted native peers (id, name, certificate fingerprint).
    Peers,
    /// List pending pairing requests (id, name, eight-digit code).
    Pending,
    /// Approve a pending pairing request after comparing codes.
    ///
    /// Compare the eight-digit code shown by `native pending` with the code
    /// on the phone before approving. Records approval; the phone still
    /// confirms the pairing.
    Pair {
        /// Pending request id from `native pending`.
        #[arg(value_name = "ID")]
        id: String,
        /// Eight-digit pairing code shown on both sides.
        #[arg(value_name = "CODE")]
        code: String,
    },
    /// Revoke a trusted native peer.
    Unpair {
        /// Trusted peer id from `native peers`.
        #[arg(value_name = "ID")]
        id: String,
    },
    /// Send a user-visible liveness ping to a native device name or ID.
    ///
    /// Queued on the phone. Use `monitor` to observe the Android result.
    Ping {
        /// Native peer name (when unique) or peer ID.
        #[arg(value_name = "DEVICE")]
        device: String,
    },
    /// Ring and vibrate a native device selected by name or ID.
    ///
    /// Queued on the phone. Use `monitor` to observe the Android result.
    Ring {
        /// Native peer name (when unique) or peer ID.
        #[arg(value_name = "DEVICE")]
        device: String,
    },
    /// Lock a native phone when device-admin access is enabled.
    ///
    /// Queued on the phone. Use `monitor` to observe the Android result.
    Lock {
        /// Native peer name (when unique) or peer ID.
        #[arg(value_name = "DEVICE")]
        device: String,
    },
    /// Ask a native phone to hold its wake lock (or release it with --release).
    ///
    /// Queued on the phone. Use `monitor` to observe the Android result.
    KeepAwake {
        /// Native peer name (when unique) or peer ID.
        #[arg(value_name = "DEVICE")]
        device: String,
        /// Release the phone wake lock instead of holding it.
        #[arg(long)]
        release: bool,
    },
    /// Ask a native phone to open its tethering settings screen.
    ///
    /// Queued on the phone. Use `monitor` to observe the Android result.
    Tethering {
        /// Native peer name (when unique) or peer ID.
        #[arg(value_name = "DEVICE")]
        device: String,
    },
    /// Send a call control action to a native phone.
    ///
    /// Call commands are accepted, not confirmed. The `place` action dials
    /// a real number and requires --confirm.
    Call {
        /// Native peer name (when unique) or peer ID.
        #[arg(value_name = "DEVICE")]
        device: String,
        /// Call action (for example: place, answer, hangup, mute).
        #[arg(value_name = "ACTION")]
        action: String,
        /// Phone number or address for actions that need one.
        #[arg(value_name = "ADDRESS")]
        address: Option<String>,
        /// Explicitly authorize placing a real phone call (required for place).
        #[arg(long)]
        confirm: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum MessagesCommand {
    /// List messaging accounts with connection state.
    Accounts,
    /// List conversations for one account.
    Conversations {
        /// Messaging account id from `messages accounts`.
        #[arg(value_name = "ACCOUNT")]
        account: String,
    },
    /// Show message history for one conversation (ACCOUNT:THREAD or THREAD).
    ///
    /// THREAD alone works only when it is unambiguous across accounts.
    /// Prints a `--cursor` line when older messages remain in the window.
    History {
        /// Conversation selector: ACCOUNT:THREAD, or bare THREAD.
        #[arg(value_name = "CONVERSATION")]
        conversation: String,
        /// Maximum messages to show.
        #[arg(long, value_name = "N")]
        limit: Option<u32>,
        /// Cursor printed by a previous history page.
        #[arg(long, value_name = "CURSOR")]
        cursor: Option<String>,
    },
    /// Send a text message (accepted, not delivered).
    ///
    /// Prints a request id. Delivery status arrives later as an attested
    /// event visible in `monitor`.
    Send {
        /// Conversation selector: ACCOUNT:THREAD, or bare THREAD.
        #[arg(value_name = "CONVERSATION")]
        conversation: String,
        /// Message text.
        #[arg(value_name = "TEXT")]
        text: String,
    },
    /// Send a file attachment with an optional caption.
    ///
    /// The path is canonicalized locally. Accepted, not delivered.
    SendFile {
        /// Conversation selector: ACCOUNT:THREAD, or bare THREAD.
        #[arg(value_name = "CONVERSATION")]
        conversation: String,
        /// Local file to attach.
        #[arg(value_name = "PATH")]
        path: PathBuf,
        /// Optional caption sent with the attachment.
        #[arg(long, value_name = "TEXT")]
        caption: Option<String>,
    },
    /// Reply to a message (ACCOUNT:THREAD:MESSAGE or THREAD:MESSAGE ...).
    ///
    /// Accepted, not delivered. The daemon validates that the target
    /// message is known before accepting.
    Reply {
        /// Message selector: ACCOUNT:THREAD:MESSAGE, THREAD:MESSAGE, or id.
        #[arg(value_name = "MESSAGE")]
        message: String,
        /// Reply text.
        #[arg(value_name = "TEXT")]
        text: String,
    },
    /// Add a reaction to a message.
    React {
        /// Message selector: ACCOUNT:THREAD:MESSAGE, THREAD:MESSAGE, or id.
        #[arg(value_name = "MESSAGE")]
        message: String,
        /// Emoji reaction.
        #[arg(value_name = "EMOJI")]
        emoji: String,
    },
    /// Remove a reaction from a message.
    Unreact {
        /// Message selector: ACCOUNT:THREAD:MESSAGE, THREAD:MESSAGE, or id.
        #[arg(value_name = "MESSAGE")]
        message: String,
        /// Emoji reaction to remove.
        #[arg(value_name = "EMOJI")]
        emoji: String,
    },
    /// Mark a conversation read (optionally up to one message).
    Read {
        /// Conversation selector: ACCOUNT:THREAD, or bare THREAD.
        #[arg(value_name = "CONVERSATION")]
        conversation: String,
        /// Message selector marking the read point.
        #[arg(long, value_name = "MESSAGE")]
        message: Option<String>,
    },
    /// Send a typing-start ping (no typing-stop exists upstream).
    Typing {
        /// Conversation selector: ACCOUNT:THREAD, or bare THREAD.
        #[arg(value_name = "CONVERSATION")]
        conversation: String,
    },
    /// Delete one own message.
    ///
    /// Accepted, not confirmed. Only own messages can be deleted.
    Delete {
        /// Message selector: ACCOUNT:THREAD:MESSAGE, THREAD:MESSAGE, or id.
        #[arg(value_name = "MESSAGE")]
        message: String,
    },
    /// Open or create a conversation with addresses (phone numbers/emails).
    ///
    /// Accepted with a request id. Watch for the conversation event; the
    /// conversation is not confirmed by this command.
    Open {
        /// Messaging account id from `messages accounts`.
        #[arg(value_name = "ACCOUNT")]
        account: String,
        /// Participant addresses (phone numbers or emails).
        #[arg(value_name = "ADDRESS")]
        addresses: Vec<String>,
    },
    /// Log in: read a credential bundle from a file or stdin, never argv.
    ///
    /// The bundle is piped, never passed as an argument, never printed,
    /// and never logged. With --from-file, reads PATH. Without it, reads
    /// stdin. Accepted means the helper took the bundle; confirm pairing
    /// on the phone.
    Login {
        /// Messaging account id to log in.
        #[arg(value_name = "ACCOUNT")]
        account: String,
        /// Read the credential bundle from PATH instead of stdin.
        #[arg(long, value_name = "PATH")]
        from_file: Option<PathBuf>,
    },
    /// Log out and revoke helper access.
    ///
    /// Queued; access ends when the helper revokes the session.
    Logout {
        /// Messaging account id to log out.
        #[arg(value_name = "ACCOUNT")]
        account: String,
    },
    /// Ask the helper to re-emit authoritative state for one account.
    Sync {
        /// Messaging account id to resync.
        #[arg(value_name = "ACCOUNT")]
        account: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum MediaSubcommand {
    /// Start playback on one session.
    Play {
        /// Session id (DEVICE:PLAYER) or unique application name.
        #[arg(value_name = "SESSION")]
        session: String,
    },
    /// Pause playback on one session.
    Pause {
        /// Session id (DEVICE:PLAYER) or unique application name.
        #[arg(value_name = "SESSION")]
        session: String,
    },
    /// Toggle playback on one session.
    #[command(name = "play-pause")]
    PlayPause {
        /// Session id (DEVICE:PLAYER) or unique application name.
        #[arg(value_name = "SESSION")]
        session: String,
    },
    /// Skip to the next item on one session.
    Next {
        /// Session id (DEVICE:PLAYER) or unique application name.
        #[arg(value_name = "SESSION")]
        session: String,
    },
    /// Return to the previous item on one session.
    Previous {
        /// Session id (DEVICE:PLAYER) or unique application name.
        #[arg(value_name = "SESSION")]
        session: String,
    },
}
