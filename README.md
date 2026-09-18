# Handover

Handover makes Android devices part of a Linux desktop. It is early-stage
infrastructure: `handoverd` owns the authoritative device, notification, media,
call, share, and messaging state, while CLI and Quickshell clients rebuild their
views from daemon snapshots.

## Components

- **`handoverd`.** The daemon owns normalized state, backend supervision, and
  Unix-socket IPC.
- **`handoverctl`.** The CLI performs one-shot inspection and control.
- **Quickshell example.** A disposable reference client in `quickshell/`.
- **Android companion.** It provides the native authenticated transport plus notification,
  media, call, and share integrations under `android/`.
- **KDE Connect backend.** It is the current compatibility backend. Its D-Bus details
  stay below Handover's public model.
- **[Google Messages adapter](https://github.com/mananlalwani/handover-gmessages)**
  is a separate AGPL-licensed production relay. It communicates with Handover
  through the coarse helper IPC contract; it is not vendored here.

Native Android currently provides authenticated presence, battery, notifications,
media, calls, and shares. KDE Connect remains the compatibility path for
capabilities not yet migrated to the native backend.

## Install and run

Install KDE Connect, pair the phone, and install Handover for the current user:

```sh
make install-user
handoverctl devices
handoverctl notifications
handoverctl media
```

The install target builds release binaries, installs them under `~/.local`, and
enables the `handoverd` user service. Ensure `~/.local/bin` is on `PATH`.

Inspect or restart the service with:

```sh
systemctl --user status handoverd
systemctl --user restart handoverd
journalctl --user -u handoverd
```

## Native Android

Build and install the companion from `android/`, then enable Handover on the
phone while both devices can reach each other:

```sh
cd android
./gradlew assembleDebug
```

The APK is `android/app/build/outputs/apk/debug/app-debug.apk`. In the app,
enable the Handover connection. On Linux:

```sh
handoverctl native pending
handoverctl native pair <pending-id> <eight-digit-code>
handoverctl native peers
```

Compare the fresh code on both devices before approving. If multicast discovery
is unavailable, enter an explicit `address:24837`; a reachable Tailscale
address is also supported. Unpairing removes the local trust record and closes
the connection on that endpoint.

See [`docs/native-interop.md`](docs/native-interop.md) for protocol details,
live verification history, and known native-backend limitations. Android
security decisions are recorded in
[`docs/android-security-review.md`](docs/android-security-review.md), and the
latest lifecycle run is in
[`docs/android-live-reliability-2026-09-17.md`](docs/android-live-reliability-2026-09-17.md).

## Clients and common commands

Run the reference Quickshell client:

```sh
handoverctl monitor
quickshell --path quickshell/example.qml
```

Reloading the client reconnects and requests a fresh daemon snapshot. The
daemon remains authoritative if a client exits or restarts.

Send a URL or file through a selected paired device:

```sh
handoverctl send-url "Phone name" https://example.com
handoverctl send-file "Phone name" /path/to/file.txt
```

An accepted command is not proof of delivery unless the backend provides a
completion result. KDE Connect currently exposes accepted-only outgoing share
semantics; native transfers provide bounded, authenticated delivery results.

The reference client displays notifications and media sessions. Controls are
capability-gated and later state is authoritative.

Clipboard synchronization remains owned by KDE Connect; Handover does not add a
second clipboard engine or persist clipboard contents.

## Google Messages

The optional production relay is maintained in the separate
[`handover-gmessages-adapter`](https://github.com/mananlalwani/handover-gmessages)
repository. Build it there, then point Handover at the resulting binary:

```sh
export HANDOVER_GMESSAGES_HELPER=/path/to/handover-gmessages-adapter
```

The adapter owns Google credentials, pairing, relay RPCs, and media transfer.
Only normalized records and opaque identifiers cross the helper boundary. See
[`docs/gmessages-sidecar.md`](docs/gmessages-sidecar.md) for the contract and
operational rules.

## Development and verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Android checks:

```sh
cd android
./gradlew assembleDebug lintDebug testDebugUnitTest
```

QML checks require `qmllint`:

```sh
qmllint quickshell/*.qml
```

To run from the source tree instead of the installed service:

```sh
systemctl --user stop handoverd
RUST_LOG=info cargo run -p handoverd
```

In another terminal, use `cargo run -p handoverctl -- devices` or
`cargo run -p handoverctl -- monitor`.

Handover is licensed under the MIT License.
