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

Command ownership is also isolated in Quickshell. Ordinary requests now use
separate connections, with at most 16 in flight and no mutation queue or replay.
A fixture reproduced a background error being mistaken for a send failure before
the fix. The expanded [coverage inventory](audits/ipc-2026-11-10.md) records
concurrent command errors, independent list assemblies, lost replies, bounded
admission, and subscription reconnect without resending actions. Settings saves
use their own completion signal rather than the global error field. Packaging
includes the new request component.

Validation passed: Rust formatting, strict workspace Clippy, workspace tests,
Android debug build/lint/JVM unit-test tasks, QML lint, all 49 JavaScript tests,
20 offscreen desktop sharing/socket scenarios, and launcher tests. These are
local and fixture checks; no physical-phone or live desktop interaction was
performed for this change. The ordinary workspace suite retains ignored tests
that require additional runtime services.

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

Local sharing hardening complete: the desktop entry offers optional HTTP/HTTPS
handling, and the send picker can request cancellation of a queued share. The
service now consumes cancellation replies and releases its pending command.
The native backend confirms cancellation only when it removes an unsent outbox
entry, preserving receiver acknowledgements after dispatch. A TLS regression
failed on the previous false-cancellation behavior before the fix.

The offscreen fixture exercises the real launcher and desktop-entry command in
14 scenarios: file paths with Unicode and reserved characters, file URLs,
HTTP/HTTPS links, offline devices, a missing daemon, rejected sends, early
completion, cancellation success/rejection/error, disconnects, and closing an
accepted transfer. Repeated activation submits once. Desktop chooser behavior
and the receiver's real effects still need live validation.

Validation passed: Rust formatting, strict workspace Clippy, workspace tests,
Android debug build/lint/JVM unit-test tasks, QML lint, all 49 JavaScript tests,
the 14 offscreen sharing scenarios, launcher tests, and desktop-file validation.
The ordinary Rust suite retains its environment-dependent ignored tests.

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

Conversation-creation investigation complete against repository sources. The
[evidence inventory](audits/gmessages-conversation-creation-2026-11-10.md) finds
the normalized command and helper contract already present, but no sufficient
first-party wire evidence for native creation or reconciliation. Native support
remains unavailable. The inventory specifies the official-client observations
needed before implementing it; no live account request or protocol mutation was
performed during this investigation.

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

## 6. Native Messages app

The selected direction is a separate Rust GTK4/libadwaita Messages app. The
first task is reading, finding, and replying to existing conversations through
public daemon IPC. The app restores daemon-owned drafts and the selected message
position. Quickshell remains a reference client.

The local implementation includes saved-message search, conflict-aware drafts,
bounded reconnect handling, native account setup presentation, and notification
actions that route into the existing app instance. Installation includes the
binary and desktop entry. See the [app guide](messages-app.md).

Automated private-display tests cover widgets and IPC with a fake daemon. Real
phone delivery, authenticated provider recovery, and interaction with an actual
desktop notification server remain separate live checks. New-conversation
creation still depends on the missing first-party protocol evidence in item 4.

AUR publication and additional distribution formats follow reproducible package
and upgrade checks; they do not block the app implementation.

## 7. Native control center

The native Linux control center uses the Messages app's visual style and groups
controls by task. A persistent phone selection scopes Phone, Notifications,
Calls, Files, Contacts, and Media. Clipboard history is computer-wide.
File selection supports review, drag and drop, and independent reported outcomes without resending unknown requests.
It brings device status, code-comparison pairing, notification actions and
replies, file and link sharing, reviewed clipboard text, synced contacts, and phone media controls into one daemon client. Ordinary
`handover-gui` launches prefer the new app; explicit QML and send-picker entry
points remain available. See the [control center guide](control-center.md).

Notification reply drafts stay in the window across updates and disconnects.
Removed notifications cannot receive a draft, but its text remains recoverable.
Lost acknowledgements never trigger automatic resends. Clipboard history reads
the daemon's computer-wide history and supports search, copy, pinning, and
confirmed clearing with pinned entries preserved by default. Physical-phone
reply effects still need live validation.

Recent transfer observations are retained by the daemon for its current session,
with a maximum of 128 records. Clients reconstruct this list through public IPC.
It is not a durable transfer archive. Calls, phone folder listings, device ring,
lock, awake requests, hotspot settings, and clipboard mirroring now have native
controls. Call requests can carry the observed generation to reject stale
controls. Media now includes laptop-audio receiving and shared application,
output, and player routing. Phone contains remote input; Device settings contains
configured commands and desktop sleep-policy requests. Contacts uses bounded
asynchronous picture decoding and offers call shortcuts. Calls exposes local HFP
observations. These close the recorded native interface gaps. Phone directory
listings remain read-only because no backend download operation exists; live
phone and audio effects still need device validation. See the guide's coverage
table.

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

Linux tarball installation and upgrade checks now cover private prefixes,
runtime assets, service commands, launchers, and failed IPC readiness. A local
release daemon also passed IPC and native listener startup in a private network
namespace. See the [package validation record](audits/linux-package-2026-11-10.md)
for the fixture boundaries and remaining distribution checks.

Ubuntu 24.04 package validation is also complete in a disposable KVM VM. It
reproduced a user-service capability failure under the default AppArmor policy;
the portable user units now start without changing that policy. Clean install,
identity-preserving reinstallation, and recovery from a failed published 0.4.0
installation passed. The CI and release jobs now exercise those boundaries.
See the [Ubuntu validation record](audits/linux-ubuntu-24.04-2026-11-10.md).

Credential-store integration now passes against private GNOME Keyring 51.1 and
46.1 instances. Real service checks cover updates, lock errors, unlock wakeups,
restart persistence, malformed and duplicate records, and deletion. CI runs the
isolated launcher explicitly. See the
[credential-store validation record](audits/credential-store-2026-11-10.md).
Authenticated Google account recovery and physical-phone behavior still need
their separate live checks.

The native Rust GTK4/libadwaita Messages app now passes its private-display GUI
checks on the host and Ubuntu 24.04. Its package passes clean installation,
reinstallation, and recovery from the pinned 0.4.0 baseline in a disposable VM.
See the [native app validation record](audits/native-messages-2026-11-11.md).
Live account and phone checks remain unverified.
