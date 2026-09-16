# Handover

Make your Android devices part of your Linux desktop.

Handover is in early development. It aims to provide Linux desktops with a
shared, desktop-level view of nearby Android devices and their capabilities.

The first backend uses KDE Connect. KDE Connect details stay behind
Handover's own domain model and APIs so frontends do not depend on its D-Bus
interfaces and other backends can be added later.

`handoverd` discovers devices through KDE Connect's session D-Bus service and
keeps their normalized connection, pairing, and battery state in memory.
It also tracks active Android notifications and their reply/dismissal support.
Clients read that state through a local Unix socket.

## Quick start

Install KDE Connect, pair your Android device, then install Handover for the
current user:

```sh
make install-user
handoverctl devices
handoverctl notifications
```

The install target builds release binaries, installs them under `~/.local`,
and enables the `handoverd` user service under `default.target`. Ensure
`~/.local/bin` is on `PATH`.

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
