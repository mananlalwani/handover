# Contributing to Handover

Handover is a Rust daemon and CLI with an Android companion and a Quickshell
reference client. The native Google Messages client and helper live here.

## Repository layout

- `handoverd/` contains the daemon and its authoritative runtime state.
- `handoverctl/` contains the CLI.
- `crates/handover-core/` contains backend-independent domain types.
- `crates/handover-ipc/` contains the Unix-socket protocol.
- `crates/handover-native/` contains the Linux native transport.
- `crates/handover-kdeconnect/` contains the KDE Connect adapter.
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

- Rust and Cargo.
- A JDK and Android SDK for Android work.
- Qt Quickshell and `qmllint` for QML work.

The Android Gradle versions are declared in the Android build files.

## Verification

Run the Rust checks from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Run the Android checks from `android/`:

```sh
./gradlew assembleDebug lintDebug testDebugUnitTest
```

Run the QML check from the repository root:

```sh
qmllint -I /usr/lib/qt6/qml quickshell/HandoverService.qml quickshell/example.qml quickshell/pages/*.qml
```

Run all checks affected by a change. Report checks that are unavailable on the
current machine.

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
