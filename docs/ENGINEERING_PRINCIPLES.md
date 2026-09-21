# Handover engineering principles

Handover must keep working when the socket disconnects, the phone reboots, or
a client is slow. Document which guarantees hold through each failure.

## Guarantees

Write the contract before the happy path.

Reconnection needs answers for which state survives, how stale sessions are rejected, whether a retry can duplicate work, and when a human has to intervene.

File transfer needs size limits, filename rules, what happens to partial files, who cleans them up, and the exact point a transfer counts as complete.

Tests should fail when those answers change.

## Invariants

Keep these unless the change is an explicit protocol or behavior review.

- An unauthenticated peer cannot produce trusted device events.
- A paired device keeps a stable identity until the user removes trust.
- Discovery never establishes trust.
- A stale session cannot mutate current state.
- A slow client cannot grow daemon memory without bound.
- Untrusted frames, files, queues, and history requests have hard limits.
- An incomplete file is never exposed as a completed transfer.
- Credentials do not appear in command-line arguments.
- Ordinary logs do not contain message bodies, tokens, keys, notification text, clipboard contents, or file contents.
- Incomplete sync cannot delete state that has not been proven stale.
- A provider crash must not take down `handoverd`.
- Accepted and completed stay different states.

## Modules

Split a file when it has separate responsibilities. Keep a connection loop
together when splitting it would make its state harder to follow. Tests can
live next to the code they cover.

Native transport now looks like this:

```text
crates/handover-native/src/
    lib.rs
    limits.rs
    errors.rs
    discovery.rs
    identity.rs
    pairing.rs
    transfer.rs
    protocol/          frame, messages, validation
    session/           connection loop
    services/          notifications, media
```

IPC handlers live under `handoverd/src/ipc_server/` and cover connection,
protocol, devices, notifications, media, calls, messaging, and transfers.

Android `NativeTransport` is split by inbound frames, clipboard, contacts, and
shares. Quickshell views live in `quickshell/pages/`, which `example.qml` imports.

## Shared helpers

Do not invent a crate-wide abstraction the first time two functions look similar. Extract bounded reads, retries, timeouts, IDs, safe names, atomic writes, correlation, session generation, error mapping, and log redaction once the rules actually match.

## Clients

First-party apps use the same daemon state and documented IPC as everyone else. A Messages app does not open its own Google session. A Calls app does not grow a second phone connection. Contacts do not keep a private database unless it is a cache of the public model.

Quickshell already reconnects and rebuilds from `handoverd`. If the public model cannot express a need, extend the model. Do not add an app-only backdoor.

Unix-socket IPC is still experimental until a versioned 1.0.

## Failure tests

Break things on purpose: cut a transfer, restart the daemon mid-sync, kill Android during pairing, duplicate frames, delay acks, fill queues, send junk or oversized payloads, revoke permissions, bounce networks, corrupt state files, die between temp write and rename.

A reproducible user bug should become a regression test when that is practical.

## Measurements

Use numbers when you claim a reliability or cost change helped: pairing and reconnect latency, idle CPU and memory, Android battery, message round-trip, transfer throughput, recovery time, queue saturation, reconnect and error rates.

## Security

Use authenticated TLS and explicit pairing. Bound frames, files, transfer
timeouts, and concurrency. Validate paths, restrict state-file permissions,
write atomically, redact logs, and isolate providers. Native received files
must not open or execute automatically.

## Git

`main` should be readable. Subjects look like `native: reconnect paired peers
after suspend`. Squash branches that are only fixups. Keep multiple commits
when each one is a real change.
