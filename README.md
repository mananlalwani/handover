# Handover

Make your Android devices part of your Linux desktop.

Handover is in early development. It aims to provide Linux desktops with a
shared, desktop-level view of nearby Android devices and their capabilities.

Handover currently supports KDE Connect and an early native Android backend.
KDE Connect details stay behind Handover's own domain model and APIs, so
frontends do not depend on its D-Bus interfaces or on the native LAN protocol.

`handoverd` discovers devices through KDE Connect's session D-Bus service and
the native backend's authenticated local-network transport. It keeps their
normalized connection, pairing, and battery state in the same daemon-owned
model. The native link currently covers presence and battery percentage with
charging state; other features remain on the KDE Connect path until separately
migrated.
It also tracks active Android notifications and their reply/dismissal support.
Clients read that state through a local Unix socket.

The production Google Messages relay lives in the separate
[`handover-gmessages-adapter`](https://github.com/mananlalwani/handover-gmessages)
repository to preserve the AGPL/MIT licensing boundary.

## Quick start

Install KDE Connect, pair your Android device, then install Handover for the
current user:

```sh
make install-user
handoverctl devices
handoverctl notifications
handoverctl media
```

The install target builds release binaries, installs them under `~/.local`,
and enables the `handoverd` user service under `default.target`. Ensure
`~/.local/bin` is on `PATH`.

For native pairing, build and install `android/app` on the phone, then open
Handover and tap "Enable Handover connection" while both devices are on the
same LAN. On Linux run `handoverctl native pending`. Compare its eight-digit
code with the phone, tap "Pair" on Android, and run
`handoverctl native pair <id> <code>` using the pending ID. The code is fresh
for every pairing attempt: if a ceremony times out or is aborted, reconnect
and compare the new code. Use
`handoverctl native peers` to list trusted phones and
`handoverctl native unpair <id>` to revoke one. The phone's "Unpair this
desktop" button clears its own trust record. `handoverctl devices` and the
Quickshell example then read the same normalized state for native and KDE
Connect devices.

If the LAN blocks multicast discovery, enter the Linux address and port
`24837` in the app's manual address field. This also works with a Tailscale
address when both devices can reach each other through Tailscale.

From `android/`, run `./gradlew testDebugUnitTest assembleDebug` to build the
companion. The debug APK is `android/app/build/outputs/apk/debug/app-debug.apk`.

Inspect or control the service with:

```sh
systemctl --user status handoverd
systemctl --user restart handoverd
journalctl --user -u handoverd
```

Monitor live device changes or launch the installed Quickshell example:

```sh
handoverctl monitor
quickshell --path ~/.local/share/handover/quickshell/example.qml
```

The example shows one phone notification with a reply field and a Dismiss
button when KDE Connect exposes those operations. It connects directly to
`handoverd`; reloading Quickshell reconstructs the current view from the daemon.
KDE Connect currently does not expose notification action labels through
D-Bus, so Handover cannot show action buttons for that backend yet.

The same example includes a small media card. It displays the first currently
playing (or otherwise available) remote session and sends supported play,
pause, previous, and next requests through `handoverd`. The CLI equivalent is
`handoverctl media`; for example, run
`handoverctl media pause "YouTube ReVanced"` when that application name is
unique, or use the printed `DEVICE_ID:PLAYER_ID` session ID. A successful control
command means KDE Connect accepted the request. The later media state update
is authoritative. On the development phone, Handover discovered simultaneous
Spotify and YouTube ReVanced sessions and observed YouTube pause/resume state
changes after Linux controls. A relative seek was accepted and the reported
position advanced. Other controls have not been verified on a phone.

### File and URL handoff

```sh
handoverctl send-url "Phone name" https://example.com
handoverctl send-file "Phone name" /path/to/an/existing-file.txt
```

`send-url` and `send-file` target one device by exact name or ID. The daemon
validates the target and file, then asks KDE Connect to send it. An accepted
command is not proof of delivery; KDE Connect does not expose outgoing
transfer progress or completion over D-Bus. The Quickshell example has the
same one-file chooser and URL field.

KDE Connect saves files shared from Android to its configured download folder.
Handover reports a transient received-share event after a file is saved. KDE
Connect opens incoming URLs with the desktop's default handler before Handover
receives the event. Handover does not execute received files or keep transfer
history. KDE Connect may itself open an incoming file when the sender requests
that behavior; Handover cannot prevent it through the current D-Bus API.

### Clipboard

KDE Connect's clipboard plugin currently owns text clipboard synchronization
between Linux and Android. Handover does not add a second clipboard engine or
store clipboard contents. Its public KDE Connect D-Bus interface can request a
Linux-to-device send, but it does not expose remote clipboard text or update
signals.

On Android 10 and later, Android restricts background clipboard reads. The
Android-to-Linux direction therefore needs KDE Connect's foreground "Send
clipboard" action unless the user grants KDE Connect its optional privileged
`READ_LOGS` path through ADB. Linux-to-Android can be automatic when the KDE
Connect clipboard plugin is enabled. Device and OEM behavior can vary.

Media sessions are normalized by Handover and remain in the daemon while
Quickshell is reloaded. KDE Connect's current desktop backend already exports
each remote player as an MPRIS service, so Handover does not add a second MPRIS
bridge; doing so would create duplicate players. Handover clients use the
versioned Unix-socket API instead.

Use `make uninstall-user` to stop and remove the user-local installation.
`cargo install --locked --path handoverd --root "$HOME/.local"` and the
equivalent command for `handoverctl` also work, but do not install or enable
the service or copy the Quickshell example.

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Run from the source tree

Install and pair KDE Connect, then start the daemon. If the user service is
already running, stop it first so both processes do not compete for the socket:

```sh
systemctl --user stop handoverd
RUST_LOG=info cargo run -p handoverd
```

In another terminal, inspect the current snapshot or monitor live changes:

```sh
cargo run -p handoverctl -- devices
cargo run -p handoverctl -- monitor
```

Run the minimal Quickshell example with:

```sh
quickshell --path quickshell/example.qml
```

The daemon keeps running if either client exits. Restarted clients reconstruct
their view from a fresh snapshot. KDE Connect can be installed with no
connected phone; paired offline devices still appear as disconnected.

Handover is licensed under the MIT License.
