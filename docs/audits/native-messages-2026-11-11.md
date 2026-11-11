# Native Messages validation, November 11, 2026

The app implementation is commit `e7ae7d7`. It adds a Rust GTK4/libadwaita
client using normalized daemon IPC. The daemon stores drafts and the selected
message position, and searches fetched history through a lazy FTS5 index.
The optional desktop bridge routes message notification actions into the app.

## Local automated checks

Passed:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`, 584 passed and seven environment-dependent tests
  ignored by default
- `cargo deny check`, including licenses and security advisories
- `python3 scripts/test-handover-messages.py`, including the explicitly invoked
  ignored GUI test
- Android `assembleDebug lintDebug testDebugUnitTest` with the installed SDK
- QML lint, 49 JavaScript tests, and the offscreen send-window fixture
- Isolated installer fixtures, shell syntax checks, and desktop-entry validation

The GUI test uses Xvfb, a private D-Bus session, temporary XDG directories, a fake
Unix-socket daemon, and a fake account-setup component. It exercises GTK widgets,
draft saving and restore, saved-message context, offline send controls, accepted
and uncertain sends, reconnect without replay, native setup progress, and two
real app processes routing into one instance.

A regression test first reproduced sends queued while disconnected carrying
into a later session. The connection worker now rejects those commands before
publishing a new connected session. The focused test passes after the fix.
The GUI test also had two races: an earlier request could satisfy its first
process check, and a transient notice could disappear before assertion. Its
checks now use a unique process target and persistent per-conversation send
state. Repeated runs passed after those changes.

The dependency audit permits the exact `Apache-2.0 WITH LLVM-exception` license
used by GTK's `target-lexicon` build dependency. No security advisory exception
was added.

`actionlint` was unavailable locally. The Rust 1.91 workspace check is configured
in CI but was not rerun locally for this change. Dependency metadata declares
no Rust version above 1.91.

## Ubuntu 24.04

A disposable KVM VM built the app against GTK 4.14.5 and libadwaita 1.5.0.
The private-display GUI test also passed there against the committed source,
including both production-binary launches. The host GUI test passed against
GTK 4.24.1 and libadwaita 1.10.0.
The tarball passed installer fixtures and real systemd user-service checks for
clean installation, reinstallation with native identity preservation, and
upgrade from the pinned published 0.4.0 archive. The baseline's existing
`218/CAPABILITIES` failure remains a failure; installing the current package
recovers it without changing the default AppArmor policy.

The local Ubuntu-built artifact is
`dist/ubuntu-24.04/handover-0.4.1-x86_64-unknown-linux-gnu.tar.gz`. Its `BUILD.txt`
records the clean source commit `e7ae7d7345424edf488c79f1153edb2a068985bb`,
glibc 2.39, and OpenSSL 3.0.13. Its SHA256 is:

```text
0f7be78d8a12f4ff689e6763cad660bfa50f601a3d96805031a88ff6cb7c934c
```

The VM was shut down after validation. The archive is a local build, not a
published release.

## Limits

These tests use synthetic account and message data. They do not verify Google
authentication, live phone delivery, browser compatibility, physical-device
behavior, audible playback, or interaction with a desktop notification server.
Notification routing uses a private D-Bus notification fixture. The account
setup dialog uses the existing setup component's sanitized status contract;
no live authentication was attempted.

The installed host daemon was neither replaced nor restarted. Its process ID
and systemd start timestamp remained unchanged throughout validation.
