# Handover user guide

Handover gives Linux desktop applications a shared view of Android devices.
The daemon keeps the current state. The CLI and Quickshell client reconnect and
read a fresh snapshot when they start.

## Install

Install Handover for your user:

```sh
make install-user
```

Make sure `~/.local/bin` is on `PATH`, then check the daemon:

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

KDE Connect is optional. Use it when you need a capability that the native
connection does not provide yet. Install KDE Connect, pair the phone in KDE
Connect, and let Handover discover the paired device. Clipboard synchronization
is currently KDE Connect-owned; Handover does not implement a native clipboard
service.

## Common commands

List devices:

```sh
handoverctl devices
```

Inspect notifications and media sessions:

```sh
handoverctl notifications
handoverctl media
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

## Google Messages

Google Messages support is optional and uses the separate
[Handover Google Messages adapter](https://github.com/mananlalwani/handover-gmessages).
Build that adapter in its own repository, then point Handover at the binary:

```sh
export HANDOVER_GMESSAGES_HELPER=/path/to/handover-gmessages-adapter
```

The adapter handles Google credentials and the phone relay. Handover receives
only normalized conversations, messages, statuses, capabilities, and opaque
identifiers. Follow the adapter's pairing runbook for account setup.

## Notifications, media, calls, and clipboard

Android notification access, media control, call control, and background access
are optional permissions. Enable only the capabilities you want to use.

Native clipboard transfer is explicit rather than a background mirror. On the
phone, use "Send current clipboard to Linux". From Linux, use:

```sh
handoverctl clipboard "Phone name" "text to put on the phone"
```

The native path limits clipboard text to 32 KiB and does not store it in daemon
state. KDE Connect may still provide its own background clipboard behavior.

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

The daemon will rebuild backend state. A native peer should reconnect without a
new pairing ceremony as long as its trust record remains and the saved endpoint
is reachable. If pairing was revoked or the Android app was reinstalled, pair
the devices again.

## Remove Handover

Remove the installed user service and binaries with:

```sh
make uninstall-user
```
