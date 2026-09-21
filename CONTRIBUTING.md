# Contributing to Handover

Handover is a Rust daemon and CLI with an Android companion and a Quickshell
reference client. The Google Messages production adapter lives in a separate Go
repository because it has a different license.

## Repository layout

- `handoverd/` contains the daemon and its authoritative runtime state.
- `handoverctl/` contains the CLI.
- `crates/handover-core/` contains backend-independent domain types.
- `crates/handover-ipc/` contains the Unix-socket protocol.
- `crates/handover-native/` contains the Linux native transport.
- `crates/handover-kdeconnect/` contains the KDE Connect adapter.
- `crates/handover-gmessages/` contains the helper contract, normalization,
  staging, and supervision code. It does not contain Google protocol code.
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
- Go for work in the separate Google Messages adapter repository.

The exact Android Gradle and Go versions are declared in their respective
repository files.

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
qmllint quickshell/*.qml
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

When testing with a physical phone, record whether each result came from a
physical device, an emulator, or automated tests. Do not commit private device
identifiers, credentials, pairing codes, message content, or live logs.

## Google Messages adapter

The production adapter is maintained in the [handover-gmessages
repository](https://github.com/mananlalwani/handover-gmessages). Changes to its
Go code belong there. Its public boundary with Handover is the helper IPC
contract described in [docs/gmessages-sidecar.md](docs/gmessages-sidecar.md).

Run the adapter checks from that repository:

```sh
go vet ./...
go test ./...
```

Do not copy or vendor its AGPL implementation or upstream Google protocol code
into this MIT repository. Keep Google protocol details, cookies, tokens, and
keys inside the adapter process. Normalized messages and staged media may cross
the helper contract under its validation and size limits.

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
