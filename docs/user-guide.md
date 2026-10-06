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

Google Messages uses the native MIT helper bundled with Handover. It still
requires Google Messages on the phone and Google's service. Confirmed native
pairings reconnect after daemon restarts using the private session record and
desktop credential store.

### Browser setup

Install the optional setup component with `./install.sh --with-google-messages-setup`
from a release archive, or `make install-user GOOGLE_MESSAGES_SETUP=1` from source.
It needs Python 3 and an installed Chromium browser. It discovers Chrome,
Chromium, Brave, Edge, Vivaldi, Opera, and Helium, prefers a supported default
browser, and lets you choose another executable. No browser is downloaded.

In Messages, click **Connect Google Messages**, choose a browser, and continue.
Sign in normally, then choose **Exit** in the browser menu. Handover reopens its
temporary browser profile briefly to finish authentication, closes that browser,
and starts native setup. If phone confirmation is needed, confirm the displayed
symbol on your phone. A saved account with matching registered device and phone
identities refreshes credentials instead of creating another pairing.

Your normal browser profile is not accessed. The temporary profile is removed
after authentication; required Google authentication is saved in desktop Secret
Service. Messages pause during setup, while other Handover features keep running.
Cancellation resumes the helper. A crashed setup's pause expires after fifteen
minutes. Interrupted registration or pairing is not retried automatically.
Check your phone's linked devices before repeating a failed attempt.

Normal Chrome passkey sign-in and the read-only native handoff have passed live
tests. The complete setup dialog, credential refresh, and fresh phone pairing
flow still need end-to-end live checks. Other detected browsers are unverified.
An account is shown connected only after the daemon reports a usable session.

For terminal setup, run `handover-google-messages-setup`. It prints setup progress
and the confirmation symbol. `--browser /path/to/browser` selects an executable;
`--list-browsers` lists detected choices without changing account state.

### Development extension

The included extension remains available for protocol diagnostics and the earlier
pairing flow. For a user installation, register its host with:

```sh
python3 "${XDG_DATA_HOME:-$HOME/.local/share}/handover/google-messages/tools/install_native_probe.py" \
  --binary "$HOME/.local/bin/handover-google-messages-auth-probe"
```

For a system package, use `/usr/share/handover/google-messages/tools/install_native_probe.py`
and `/usr/bin/handover-google-messages-auth-probe` instead. Load the adjacent
`chrome-observer` directory with Chrome's **Load unpacked** action, then follow
its [native registration and pairing instructions](../google-messages/tools/chrome-observer/README.md#register-one-native-device).
The phone confirmation is required once. Keep Google Messages web tabs closed
while using Handover's connected native session.

The daemon receives normalized conversations, messages, statuses, capabilities,
and opaque identifiers. Google authentication and keys stay inside the native
helper and desktop credential store. Login bundles are sensitive; never put
one in command arguments, shell history, logs, or a committed file. Native
logout removes local credentials; removing the linked device on the phone is
still a separate step.

### Optional legacy relay

The separate AGPL-3.0-only
[legacy relay](https://github.com/mananlalwani/handover-gmessages) remains
available. To select it explicitly, use `systemctl --user edit handoverd`:

```ini
[Service]
Environment=HANDOVER_GMESSAGES_HELPER=/path/to/handover-gmessages
```

Restart with `systemctl --user restart handoverd`, then follow the legacy
[setup guide](https://github.com/mananlalwani/handover-gmessages/blob/main/docs/user-guide.md).
An existing explicit override continues to take precedence after upgrades.
Removing it restores native discovery. A missing override or a failed native
session does not silently choose another provider.

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
retain their MIME type. On Android, tap "Send file to desktop" to choose one
file, or share a file or URL from another app to Handover. The phone must be
connected to its paired desktop. Check Activity > Recent transfers for the
receiver's result. A queued transfer is not a delivery confirmation.

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
