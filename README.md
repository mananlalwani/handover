# Handover

Handover connects Android devices to a Linux desktop. `handoverd` owns the
current device state, while `handoverctl` and the Quickshell example display or
control it.

The project currently supports KDE Connect and an early native Android
transport. The native transport uses authenticated TLS and keeps its trust
decision tied to an explicit pairing ceremony.

## Use Handover

Read the [user guide](docs/user-guide.md) for installation, pairing, common
commands, troubleshooting, privacy notes, and optional Google Messages setup.

## Project documentation

- [`DESIGN.md`](DESIGN.md) describes the architecture and backend boundaries.
- [`docs/gmessages-sidecar.md`](docs/gmessages-sidecar.md) defines the separate
  Google Messages helper contract.
- [`docs/native-threat-model.md`](docs/native-threat-model.md) records the
  native transport's security assumptions.
- [`docs/native-backend-apis.md`](docs/native-backend-apis.md) records Android
  API decisions behind the native transport.
- [`docs/native-calls.md`](docs/native-calls.md) describes native call support.
- [`quickshell/README.md`](quickshell/README.md) documents the reference client.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Android checks run from `android/`:

```sh
./gradlew assembleDebug lintDebug testDebugUnitTest
```

Handover is licensed under the MIT License. The production Google Messages
adapter is maintained separately at
<https://github.com/mananlalwani/handover-gmessages> because it has a different
license boundary.
