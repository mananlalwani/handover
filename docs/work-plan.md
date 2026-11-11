# Handover work plan

This is the execution order for the gaps recorded on November 10, 2026.
Milestones 1 and 2 remain complete as scoped in the [roadmap](../ROADMAP.md).
Their completion does not replace device validation or make the public IPC stable.

## 0. Resolve the dependency audit failure

The initial local baseline failed `cargo deny check` on `RUSTSEC-2026-0459` in
`libsqlite3-sys 0.38.2`, reached through the daemon's bundled `rusqlite 0.40.2`.
The advisory describes a crafted database/WAL-file attack; this result alone does
not demonstrate exploitation through Handover.

- Verify the advisory and patched dependency options against upstream sources.
- Use a published patched dependency and declare its supported Rust minimum
  explicitly. Do not suppress the advisory.
- Run the archive restart, paging, removal, and size-limit tests, the workspace
  checks, and the dependency audit after the upgrade.

Done when the patched dependency builds under the declared toolchain policy,
archive behavior is preserved, and `cargo deny check` passes.

Resolved: upgraded to `rusqlite 0.41.1` and `libsqlite3-sys 0.39.1`, which bundles
SQLite 3.54.1. This is a patched version according to
[RustSec](https://rustsec.org/advisories/RUSTSEC-2026-0459.html). The workspace
minimum is explicitly raised from Rust 1.88 to 1.91, and CI checks all workspace
targets with Rust 1.91.0. The SQLite wrapper requires 1.89, but testing that
toolchain exposed the existing `rtc-mdns 0.21.1` dependency's use of
[`Ipv4Addr::from_octets`](https://doc.rust-lang.org/std/net/struct.Ipv4Addr.html#method.from_octets),
which stabilized in 1.91. No advisory exception was added. The lockfile update
changes only the two SQLite packages.

Validation passed: the five archive tests, Rust formatting, strict workspace
Clippy, the full workspace suite on both Rust 1.91.0 and the installed 1.99.1,
the exact minimum-version CI check, and an isolated systemd startup smoke test.
`cargo deny --locked check` reports advisories, bans, licenses, and sources all
passing. Environment-dependent tests remain ignored by the ordinary suite.

## 1. Validate existing workflows

Start with the current implementation before adding capabilities. A fixture
passing is evidence about that fixture, not evidence that a phone performed an
action.

- Run the Rust, Android, QML, launcher, and desktop fixture checks in
  [Contributing](../CONTRIBUTING.md). Record failures against the tested commit.
- Run the [phone checklist](phone-validation.md) in this order: file and URL
  sharing in both directions; disconnect during a transfer; notification actions
  and MPRIS controls; suspend/resume and network changes; Google Messages session
  recovery; audible laptop audio.
- Run the [Android security checks](../android/SECURITY_DEVICE_VALIDATION.md)
  before calling the build ready for a release.
- For each failure, capture a minimal reproduction and add a regression test at
  the boundary that failed. Fix false success, duplicate actions, trust failures,
  and lost state before cosmetic defects.

Done when automated checks pass and each device scenario has a recorded pass,
failure, or explicit untested status with build and environment details. Keep
private logs and device identifiers out of committed results. Release readiness
requires resolving blocking failures; an untested scenario does not count as a
pass.

First slice: establish the local automated baseline. Physical-device work needs
a connected test phone; neither paired device was online and ADB listed no device
when this work began.

## 2. Establish an IPC compatibility baseline

The [IPC contract](ipc.md) already documents snapshot recovery, size limits,
slow-client behavior, and command uncertainty. Audit existing coverage before
adding tests or changing those semantics.

- Map each documented rule to existing Rust, CLI, desktop, and Quickshell tests.
- Fill behavioral gaps for interrupted snapshots, lag recovery, unsupported
  versions, partial request reads, and disconnect after mutation acceptance.
- Verify clients replace snapshots atomically and never resend mutations during
  reconnect. Check bounded memory and continued service to other clients when a
  subscriber stalls.
- Document the compatibility policy for optional fields, new enum values,
  breaking changes, and protocol version negotiation, then implement only changes
  the audit shows are necessary.

Done when the policy and tests agree across shipped clients, with no backend
protocol details in public models. Keep protocol 1 experimental until that work
supports a stability commitment. A protocol-number change is not itself a fix.

Dependency: the automated baseline from item 1. Device testing can continue
alongside this work.

Local implementation complete: the [coverage inventory](audits/ipc-2026-11-10.md)
records the retained tests and new boundary coverage. Quickshell now bounds and
assembles all complete-list variants before publishing them. Daemon lag recovery
discards superseded events before taking a replacement snapshot. Both defects
were reproduced with failing regressions before their fixes. The
[compatibility policy](ipc.md#compatibility-policy) documents protocol evolution
without declaring protocol 1 stable.

## 3. Finish everyday Linux integration

Extend the existing send picker, notification bridge, and MPRIS adapter before
building a larger application.

- Validate file-manager and browser entry points with Unicode paths, multiple
  files, offline devices, cancellation, and an unavailable daemon.
- Make device selection and transfer outcomes understandable in the existing
  views. Show accepted, confirmed completion, failure, and unknown outcomes only
  where the backend supplies that evidence.
- Close usability defects found in item 1, including notification dismissal and
  media-control behavior after reconnect.

Done when a user can send files and links through ordinary desktop entry points,
see an honest outcome, and recover from an offline phone without restarting a
client. Verify in an actual desktop session as well as fixtures.

Dependencies: item 1's sharing and desktop checks, plus any relevant IPC fixes.

## 4. Add messaging capabilities one at a time

Current gaps include conversation creation, mark-read, deletion, and outgoing
typing. Support must follow first-party protocol evidence, the
[helper contract](gmessages-helper.md), and actual account capabilities.

- Investigate conversation creation first because it enables starting a message
  without an existing thread. Follow with mark-read, deletion, and outgoing typing.
- For each action, establish its request, response, failure, and reconciliation
  behavior before implementing it. If evidence is insufficient, retain the
  unsupported capability and record what is missing.
- Keep Google wire types and authentication inside the native helper. Reuse
  normalized operations where they fit; extend public models only when required.
- Add helper and workspace regression coverage, then verify on a paired account.
  Do not infer remote deletion from the existing incoming tombstone support or
  advertise success from command acceptance alone.

Done per action when capability reporting, UI availability, errors, and observed
effects agree. This is four separate implementation slices, not one feature PR.

Dependencies: item 1's session-recovery validation and item 2's compatibility
policy before public contract changes.

## 5. Validate and document message retention

Persistence already exists in
[`messaging_cache.rs`](../handoverd/src/messaging_cache.rs) and
[`messaging_archive.rs`](../handoverd/src/messaging_archive.rs). The daemon stores
normalized state and fetched history pages across restarts. This does not promise
a complete provider archive. The earlier limitations text understated this support.

- Document the existing cache opt-out, archive size limit, storage location, and
  behavior when storage is unavailable or full. Check those statements against
  runtime behavior before publishing operational instructions.
- Audit existing tests for restart, account removal, confirmed deletion, paging,
  local access protection, and storage exhaustion. Add only missing behavioral
  coverage and fix demonstrated defects.
- Decide whether users need explicit retention or purge controls beyond the
  existing size setting. Keep daemon ownership and opaque provider cursors.

Done when the documented storage behavior matches the implementation and tests,
and any added controls have explicit privacy and disk-limit semantics.

Dependency: item 0 before shipping archive changes. Existing storage can be
audited before item 4; new deletion actions must reconcile with stored history.

Local audit complete: [message storage and recovery](message-storage.md) documents
existing persistence, size settings, opt-out behavior, storage failures, and
privacy limits. Existing restart, removal, and paging coverage was retained.
A new bounded-storage test fills the archive through real history-page writes
and confirms failed transactions preserve the previous page across restart.
No new retention, eviction, or purge policy was introduced.

## 6. Choose the desktop client scope

Quickshell remains a reference client. Use the validated workflows to decide
whether an installable shell client or a separate Messages, Calls, or Contacts
app solves a remaining need. Avoid building all three speculatively.

First slice: identify the task the current desktop integrations cannot complete
and choose one client to address it. Define installation, accessibility,
reconnect, and error behavior. A stable client rebuilds its state from daemon
snapshots and never owns another phone session.

Done when the selected client completes that task through public IPC and passes
desktop-session validation. AUR publication and additional distribution formats
follow a reproducible installation and upgrade test; they do not block core fixes.

Dependencies: items 2 and 3. A messaging client also depends on whichever actions
from item 4 its chosen task requires.

## Limits to keep explicit

Android background clipboard restrictions, multicast discovery across networks,
unsupported phone media controls, and the Google service dependency remain
documented constraints. Prefer a clear manual clipboard action, manual address
entry, and accurate capability reporting over claiming to remove those limits.
See [known limitations](KNOWN_LIMITATIONS.md).

## Initial local baseline

Checked November 10, 2026 against `9a90d47`, with documentation-only changes in the
working tree. These results are local automated checks, not phone validation.

- Passed: `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, loopback helper build,
  and `cargo test --workspace`. The suite retains an ignored test; a passing run
  does not verify ignored scenarios.
- Passed: Android `assembleDebug lintDebug testDebugUnitTest`, with
  `ANDROID_HOME` pointing to the installed SDK. Gradle reused existing outputs
  for most tasks. The initial invocation without an SDK location failed.
- Passed: QML lint, all 43 JavaScript tests, the offscreen send-window fixture,
  and desktop launcher tests.
- Failed: `cargo deny check`. Advisories failed on `RUSTSEC-2026-0459`; bans,
  licenses, and sources passed. Duplicate dependency warnings also remain.
- Not run: connected Android instrumentation, physical-phone validation, audible
  playback, and live desktop interaction. ADB listed no connected device.

## Follow-up validation

The local implementation checks on November 10 passed Rust formatting, strict
workspace Clippy, and workspace tests. QML lint, all 46 JavaScript tests, the
offscreen send-window fixture, and launcher tests passed. The workspace suite
still skips its environment-dependent ignored tests by default.

Android build, lint, and JVM unit-test tasks passed, followed by all seven
framework instrumentation tests on the existing Android 15 x86_64 test emulator.
The emulator ran headlessly with a temporary read-only session and was stopped
afterward. This is emulator evidence, not physical-device validation.

The isolated systemd smoke test passed startup, private socket permissions,
CLI access, a separate native listener, and mDNS setup. The explicitly invoked
native pre-auth connection-limit and live multicast-discovery tests also passed.
The installed user daemon was not replaced or restarted.

The dependency audit initially failed only the SQLite security advisory; the
upgrade recorded above resolves that failure. Physical-phone behavior, audible
audio routing, the desktop credential-store integration, and live desktop
interaction remain unverified.
