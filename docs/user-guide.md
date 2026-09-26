# Handover user guide

Linux apps talk to `handoverd` for Android devices. The CLI and Quickshell
reconnect and read a fresh snapshot when they start.

## Install

Tagged releases include a Linux tarball. Unpack it and run `./install.sh`.

Or install from source:

```sh
make install-user
```

Put `~/.local/bin` on `PATH`, then check the daemon:

```sh
systemctl --user status handoverd
handoverctl devices
```

Start the reference client with:

```sh
quickshell --path ~/.local/share/handover/quickshell/example.qml
```

## Native Android connection

The native connection is the primary connection path. Build the companion from
the repository:

```sh
cd android
./gradlew assembleDebug
```

Install `android/app/build/outputs/apk/debug/app-debug.apk` on the phone. Open
Handover and enable the connection while the phone and Linux machine can reach
each other.

On Linux, get the pending pairing request:

```sh
handoverctl native pending
```

Compare the eight-digit code shown on Linux with the code shown on the phone,
then approve the same request:

```sh
handoverctl native pair <pending-id> <eight-digit-code>
```

List trusted native peers with `handoverctl native peers`. To revoke a peer,
use `handoverctl native unpair <peer-id>` and remove the connection from the
phone when needed.

If multicast discovery does not work, enter the Linux address and port `24837`
in the Android app. A reachable Tailscale address works as well.

## KDE Connect compatibility backend

KDE Connect is optional. Use it only if you still want a capability that lives
there. Install KDE Connect, pair the phone in that app, and let Handover
discover it. Native clipboard is documented below. KDE Connect may still run
its own background clipboard sync if you keep that backend.

## Common commands

The [CLI reference](cli/handoverctl.md) links to the command pages for
[native pairing](cli/native.md), [messaging](cli/messages.md), [sharing and
clipboard](cli/sharing-clipboard.md), and [operations](cli/operations.md).
The installed man page is `handoverctl(1)`.

List devices:

```sh
handoverctl devices
```

Inspect notifications and media sessions:

```sh
handoverctl notifications
handoverctl media
handoverctl contacts list
```

Send a URL or file to one device selected by name or ID:

```sh
handoverctl send-url "Phone name" https://example.com
handoverctl send-file "Phone name" /path/to/file.txt
```

An accepted command means the selected backend accepted the request. It does
not always mean that the phone received the item or that playback changed.
Later state and completion events are authoritative when the backend provides
them.

Monitor changes as they arrive:

```sh
handoverctl monitor
```

## Idle and sleep

By default, Handover prevents idle locking and sleep while a paired native
phone is connected. Set `HANDOVER_INHIBIT_ON_CONNECT=0` in the daemon's
environment to let the desktop's idle policy run while connected. Explicit
keep-awake requests from the phone and `handoverctl screensaver inhibit` still
hold the inhibitor.

## Google Messages

Google Messages support is optional and uses the separate
[Handover Google Messages adapter](https://github.com/mananlalwani/handover-gmessages).
Build that adapter in its own repository, then point the daemon at the binary.
For a systemd user service, add the variable with `systemctl --user edit
handoverd`:

```ini
[Service]
Environment=HANDOVER_GMESSAGES_HELPER=/path/to/handover-gmessages
```

Then restart the daemon:

```sh
systemctl --user restart handoverd
```

The adapter handles Google credentials and the phone relay. Handover receives
only normalized conversations, messages, statuses, capabilities, and opaque
identifiers. Follow the adapter's [user guide](https://github.com/mananlalwani/handover-gmessages/blob/main/docs/user-guide.md)
and [pairing runbook](https://github.com/mananlalwani/handover-gmessages/blob/main/docs/pairing-runbook.md)
to obtain the login data and confirm the phone pairing. Pass the data through
standard input or `--from-file`, for example:

```sh
handoverctl messages login gmessages:personal --from-file /path/to/bundle.json
```

The login bundle is sensitive. Do not put it in command arguments, shell
history, logs, or a committed file. A saved session survives daemon restarts;
run `handoverctl messages logout gmessages:personal` to revoke it and remove
the local session.

## Notifications, media, calls, and clipboard

Android notification access, media control, call control, and background access
are optional permissions. Enable only the capabilities in use.

Native clipboard transfer is explicit rather than a background mirror. On the
phone, use "Send current clipboard to Linux". From Linux, use:

```sh
handoverctl clipboard "Phone name" "text to put on the phone"
# Or read the current Wayland clipboard:
handoverctl clipboard "Phone name"
```

The native path limits each clipboard text value to 32 KiB. Text received from
the phone is kept in the daemon's bounded clipboard history, with up to 25
recent and 25 pinned entries, and is stored at
`${XDG_STATE_HOME:-$HOME/.local/state}/handover/clipboard-history.json`.
KDE Connect may still provide its own background clipboard behavior.
An opt-in Android setting can mirror text clipboard changes while the Handover
foreground service is active. It uses content hashes to avoid echoing a change
back to its origin.

Native transfers also carry HTML and URI clipboard data. Image and other
file-backed clipboard items up to 10 MiB use the authenticated file stream and
retain their MIME type. Android share-sheet actions can send URLs and files to
the paired desktop.

The Android app also provides presentation controls, a pointer pad, and Linux
volume controls. These commands report acceptance of the request; they do not
claim that the presentation or volume changed.

## Privacy and security

- Native pairing stores the peer certificate fingerprint and requires TLS.
- The Android identity private key stays in Android Keystore.
- Native received files are size-limited and written through controlled storage
  paths.
- Google Messages credentials stay with the separate adapter. They do not cross
  into Handover's normalized state.
- Handover logs identifiers, counts, and state transitions, not message bodies,
  notification text, URLs, tokens, keys, or file contents.

## Troubleshooting

Check the daemon and recent logs:

```sh
systemctl --user status handoverd
journalctl --user -u handoverd --since today
```

Restart the daemon when its client socket or backend connection is stuck:

```sh
systemctl --user restart handoverd
```

The daemon rebuilds backend state. A native peer reconnects without a new
pairing ceremony while its trust record remains and the saved endpoint is
reachable. If pairing was revoked or the Android app was reinstalled, pair
again.

## Remove Handover

Remove the installed user service and binaries with:

```sh
make uninstall-user
```

Uninstalling leaves local state on disk. The following command deletes all
Handover state at this location, including pairing records, clipboard history,
cached messages, adapter sessions, and received files. Back up anything you
want to keep first:

```sh
rm -rf "${XDG_STATE_HOME:-$HOME/.local/state}/handover"
```

The Android app keeps its own identity in app storage and Keystore. Uninstall
the Android package separately if you want that side gone too.

See [known limitations](KNOWN_LIMITATIONS.md) for current gaps.
