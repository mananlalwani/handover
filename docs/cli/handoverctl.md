# handoverctl command reference

`handoverctl` is the command line client for Handover. It connects to the
local `handoverd` daemon, reads a fresh snapshot, and sends commands. The
daemon owns authoritative state. This page lists every command with its
arguments, prerequisites, and what success means.

Conventions used below:

- DEVICE means a device name (when unique) or a device ID. IDs are stable.
  Names are matched exactly and rejected when two devices share a name.
- Accepted means the daemon or backend took the request. It does not mean
  the phone acted on it. Watch `handoverctl monitor` for the later result.
- Every command needs a running `handoverd` reachable over
  `$XDG_RUNTIME_DIR/handover/handoverd.sock`, except `--help` and
  `--version`. Without the socket, the command exits 1.
- Exit status is 0 on success and 1 on any error.

Related pages:

- `docs/cli/native.md` for pairing and phone commands.
- `docs/cli/messages.md` for messaging accounts and conversations.
- `docs/cli/sharing-clipboard.md` for shares, clipboard, and desktop helpers.
- `docs/cli/operations.md` for daemon lifecycle, environment variables,
  exit status, and troubleshooting.
- `docs/man/handoverctl.1` for the installed man page source.

## devices

```
handoverctl devices
```

Lists devices known to `handoverd`: name, connection state, pairing state,
and battery. No arguments.

## notifications

```
handoverctl notifications
```

Lists active remote notifications from the daemon snapshot: device, app,
and title. No arguments.

## contacts

```
handoverctl contacts list
handoverctl contacts sync DEVICE
```

`list` prints the latest contacts snapshot held by the daemon. `sync`
asks one native phone to send a fresh snapshot. The sync request is
accepted, not confirmed. Arrival of the snapshot is not reported by this
command.

## media

```
handoverctl media
handoverctl media play SESSION
handoverctl media pause SESSION
handoverctl media play-pause SESSION
handoverctl media next SESSION
handoverctl media previous SESSION
```

With no subcommand, prints current media sessions: session id, device,
app, playback state, title, and artist. With a subcommand, sends one
playback command. The command is accepted by the daemon. A change in
playback is not confirmed.

SESSION is `DEVICE:PLAYER`, or the player ID or application name when
only one session uses it. Ambiguous names are rejected with a hint to
use the session id.

## monitor

```
handoverctl monitor
```

Subscribes to daemon events. Prints the initial device and notification
counts, then prints each event until interrupted with Ctrl-C. When the
daemon is unavailable, prints the error and retries after a short delay.

## calls

```
handoverctl calls DEVICE
```

Prints current normalized call state for one device: call phase and
available controls. Prints a notice when no call state is attested for
the device.

## Full command list

Top level: `devices`, `native`, `notifications`, `contacts`, `media`,
`monitor`, `send-url`, `notify`, `clipboard`, `send-file`, `screensaver`,
`clipboard-mirror`, `clipboard-history`, `cancel-share`, `custom`,
`calls`, `messages`.

Native: `peers`, `pending`, `pair`, `unpair`, `ping`, `ring`, `lock`,
`keep-awake`, `tethering`, `call`. See `docs/cli/native.md`.

Contacts: `list`, `sync`.

Media: `play`, `pause`, `play-pause`, `next`, `previous`.

Clipboard history: `list`, `save`, `pin`, `unpin`, `copy`, `clear`.

Custom: `list`, `run`.

Messages: `accounts`, `conversations`, `history`, `send`, `send-file`,
`reply`, `react`, `unreact`, `read`, `typing`, `delete`, `open`,
`login`, `logout`, `sync`. See `docs/cli/messages.md`.

Sharing, clipboard, and desktop helpers: `send-url`, `notify`,
`clipboard`, `send-file`, `screensaver`, `clipboard-mirror`,
`clipboard-history`, `cancel-share`, `custom`. See
`docs/cli/sharing-clipboard.md`.
