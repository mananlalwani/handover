# Contributing to Handover

Handover is a Rust daemon and CLI with an Android companion and a Quickshell
reference client and a native GTK4/libadwaita Messages app. The native Google
Messages client and helper live here.

## Repository layout

- `handoverd/` contains the daemon and its authoritative runtime state.
- `handoverctl/` contains the CLI.
- `handover-messages/` contains the native Rust GTK4/libadwaita app.
- `handover-desktop/` contains the optional Linux desktop notification client.
  Its integration tests start a private session bus and need `dbus-daemon`
  and `playerctl`. They do not use the real desktop session or a phone.
- `crates/handover-core/` contains backend-independent domain types.
- `crates/handover-ipc/` contains the Unix-socket protocol.
- `crates/handover-native/` contains the Linux native transport.
- `crates/handover-gmessages/` contains the helper contract, normalization,
  staging, and supervision code. It does not contain Google protocol code.
- `crates/handover-google-messages/` contains the independent Google Messages
  protocol client and its daemon-supervised helper process. It depends on the
  helper contract, never the reverse.
- `android/` contains the Android companion.
- `quickshell/` contains the reference client.

Read [DESIGN.md](DESIGN.md) before changing a public model, IPC method, or
backend boundary.

Ordinary bugs go to GitHub Issues. Security reports go through
[SECURITY.md](SECURITY.md), not a public issue.

## Prerequisites

Install:

- Rust 1.91 or newer and Cargo.
- A C/C++ toolchain, CMake, pkg-config, and OpenSSL development headers for Linux builds.
- GTK 4.10 and libadwaita 1.4 or newer, with development headers. On Ubuntu
  24.04, install `libgtk-4-dev libadwaita-1-dev`.
- `cargo-deny` for dependency advisory, license, and source checks.
- A JDK and Android SDK for Android work.
- Qt Quickshell and `qmllint` for QML work.

The Android Gradle versions are declared in the Android build files.

## Verification

Run the Rust checks from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
```

Validate Google Messages credential storage against a real GNOME Keyring service:

```sh
python3 scripts/test-secret-service.py
```

This requires `dbus-daemon` and `gnome-keyring-daemon`. The launcher builds the
ignored integration test, creates a private bus with service activation disabled,
and gives GNOME Keyring temporary storage. It checks dummy credential updates,
lock/unlock notifications, persistence across service restart, deletion, and
invalid or duplicate records. It neither uses the desktop keyring nor contacts
Google. Run this launcher rather than invoking the ignored test on a desktop bus.

Validate the native Messages app and control center without a phone or desktop account:

```sh
python3 scripts/test-handover-messages.py
python3 scripts/test-handover-control.py
```

This requires `Xvfb` and `dbus-daemon`. It starts a private display, session bus,
state directories, and fake daemon. It exercises real GTK widgets and IPC,
including draft restore, search navigation, reconnects, lost send replies, and
single-instance application routing. The setup dialog uses a fake setup helper.
The control center runner covers pairing comparison, notification actions,
sharing, contact copy, and reconnect behavior with fictional devices.
Run these launchers rather than invoking the ignored GUI tests on your desktop.

CI compiles workspace tests once and passes their Cargo JSON manifest through
`HANDOVER_TEST_ARTIFACTS` to the credential and GUI runners. Without that variable,
the runners build their own test executable as before. A supplied manifest must
contain an existing executable for the requested test target; invalid manifests
fail rather than triggering another build. The Messages runner also builds its
ordinary application binary for the single-instance checks.

For visual checks, add `--screenshot /tmp/messages.png`, optionally with
`--dark` or `--width 760`. PNG capture needs ImageMagick `import` and contains
only the synthetic conversation on the private display.

Run the Android checks from `android/`:

```sh
./gradlew assembleDebug lintDebug testDebugUnitTest
```

Run the QML check from the repository root:

```sh
qmllint -I /usr/lib/qt6/qml quickshell/IpcRequest.qml quickshell/HandoverService.qml quickshell/example.qml quickshell/send.qml quickshell/pages/*.qml
```

The JavaScript checks and offscreen send-window fixture also run without a phone:

```sh
node --test quickshell/tests/*.test.mjs
python3 scripts/test-handover-send.py
```

The [phone validation handoff](docs/phone-validation.md) covers the remaining
physical-device and audible-playback checks. Run all checks affected by a change.
Report checks that are unavailable on the current machine.

Check Linux installer behavior without changing the installed user services:

```sh
python3 scripts/test-install-from-dist.py
./scripts/dist.sh
python3 scripts/test-install-from-dist.py --archive dist/handover-0.4.2-x86_64-unknown-linux-gnu.tar.gz
```

The installer tests use private directories and service/IPC fixtures. They check
file placement, upgrades, launcher arguments, custom prefixes, and bounded IPC
readiness. They do not start the packaged daemon or validate phone behavior.
Pass `--previous-archive PATH` to check an upgrade from an existing tarball too.
Archive names follow the version and host triple printed by `dist.sh`.

For real installed-service checks, use a disposable Ubuntu 24.04 VM or CI runner
with `dbus-user-session`, `iproute2`, Python 3.12, and a running systemd system
manager. Native TCP port 24837 must be free. Build there with `build-essential`, `cmake`,
`pkg-config`, `libssl-dev`, `libgtk-4-dev`, and `libadwaita-1-dev`, then run:

```sh
sudo python3 scripts/test-package-runtime.py --disposable-environment \
  --archive dist/handover-0.4.2-x86_64-unknown-linux-gnu.tar.gz \
  --previous-archive /path/to/previous-release.tar.gz
```

This test creates temporary users and their systemd user managers, runs the
real installer and daemon, then removes the users. It checks clean installation
with a custom prefix, reinstallation, native listener ownership, IPC permissions,
preserved native identity, and replaced client assets. An earlier package that
fails with `218/CAPABILITIES` remains a failed baseline; the upgrade must recover
it. Identity preservation is still checked through a running current package.
Omitting `--previous-archive` checks reinstallation of the current build.
CI and release jobs use the published 0.4.0 tarball with a fixed SHA256 checksum.

## Running from the source tree

Stop the installed daemon before starting a source-tree instance:

```sh
systemctl --user stop handoverd
RUST_LOG=info cargo run -p handoverd
```

In another terminal, inspect the daemon through the source-tree CLI:

```sh
cargo run -p handoverctl -- devices
cargo run -p handoverctl -- monitor
```

The daemon owns state. Clients reconnect and ask for a fresh snapshot. They do
not keep their own authoritative copy.

## Android development

Build the debug APK from `android/`:

```sh
./gradlew assembleDebug
```

The output is
`android/app/build/outputs/apk/debug/app-debug.apk`. Native transport changes
must preserve the certificate identity and pairing model. Do not replace
certificate checks with permissive trust or disable TLS verification.

GitHub releases build `assembleRelease` with a dedicated signing key. The
release workflow needs `HANDOVER_ANDROID_KEYSTORE_B64`,
`HANDOVER_ANDROID_STORE_PASSWORD`, `HANDOVER_ANDROID_KEY_ALIAS`, and
`HANDOVER_ANDROID_KEY_PASSWORD` as repository secrets. Keep the keystore and
password backed up outside this repository. A lost signing key prevents Android
from installing later releases over the current app.

When testing with a physical phone, record whether each result came from a
physical device, an emulator, or automated tests. Do not commit private device
identifiers, credentials, pairing codes, message content, or live logs.

## Google Messages client

The native protocol client lives in `crates/handover-google-messages`. It uses
[helper IPC v1](docs/gmessages-helper.md) to publish normalized state to the daemon.
Keep Google protocol details, cookies, tokens, and keys inside the helper process.
Normalized messages and staged media cross the contract under its validation and
size limits. Run the Rust workspace checks for client and contract changes.

## Code and review expectations

- Keep public models and IPC semantics independent of backend names and
  protocols.
- Treat daemon snapshots as authoritative after reconnects and restarts.
- Report command acceptance separately from delivery or eventual effect.
- Bound queues, retries, frame sizes, and stored windows.
- Use established cryptographic libraries and keep secrets out of logs.
- Add tests for changes to validation, protocol parsing, recovery, trust, or
  delivery status.
- Keep changes focused and explain any behavior that remains unverified.

Use `area: imperative` subjects, matching `git log` on `main`. Do not commit
generated build output, credentials, local state, or private test data.
