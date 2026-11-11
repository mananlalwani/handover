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

## Visual redesign follow-up

The native app now uses separate sidebar and conversation headers, contact
initials, unread dots, compact conversation rows, and observed message previews.
Incoming and outgoing messages use rounded bubbles. Sender grouping and date
markers reduce repeated labels. Reply buttons appear on hover or keyboard
focus, and timestamps remain available in bubble tooltips. The composer grows
with multiline text and shows draft warnings when needed. Colors follow the
system appearance and accent color.

The private-display test passed in light and dark appearances and at a 760-pixel
window width. Synthetic screenshots were inspected for wrapping, contrast,
sidebar selection, unread indicators, and composer placement. Screenshot capture
is optional and needs ImageMagick. These images contain only fixture data.

Rust formatting, strict workspace Clippy, and the full workspace suite passed,
with 585 tests passing and seven ignored by default. The native app's focused
suite also passed, including the new preview test for older history, deletions,
and account removal. Android and QML sources were unchanged in this follow-up.

A local release build of the redesigned app replaced the installed app binary.
The existing window closed through its normal draft-saving path, then the new
window opened successfully on Hyprland. The daemon process and start timestamp
remained unchanged during this app update. The earlier Ubuntu tarball remains
the initial app build documented above; this follow-up installed the app built
on the host.

## Dense conversation list and saved previews

A second visual pass uses a denser fixture with ten conversations, including
phone-only contacts, groups, long titles, consecutive messages, and a long URL.
The earlier small fixture hid empty previews and punctuation appearing as
phone-number initials.

The app now reads saved previews through `messages.previews`. This normalized
IPC method accepts at most 50 conversation identities and returns the last
saved message for each available conversation. It reads the local archive and
does not fetch provider history or mark conversations read. The app warms
previews in batches of 20, one request at a time, for up to 512 recent
conversations per connection. Its preview cache and attempt set are bounded.
Cached history can be incomplete; rows without a saved message show their
reported transport instead of invented message content.

The layout has a left-aligned contact header, native colored avatars with
person/group fallbacks, a stronger selected row, account filtering in the header,
a participant details popover, and a separate attachment control beside the
composer. The setup window uses a native browser row and a shorter instruction
layout. Setup execution, credential handling, and phone verification semantics
remain unchanged.

Passed formatting, strict workspace Clippy, and the full Rust workspace suite,
with 588 passed and seven ignored by default. Eight focused local-message IPC
tests passed, including preview account scoping, invalid identities, batch
limits, empty conversations, newest saved messages, and large chunked replies.
The GUI fixture passed light, dark, and 760-pixel layouts, including setup,
reconnects, uncertain sends, and single-instance routing. Android build, lint,
and unit checks passed; QML lint and all 49 JavaScript tests passed.

The matching host-built daemon, CLI, desktop bridge, and app binaries were
installed after backing up the previous binaries. The old app closed through
its normal window-close flow before the daemon restarted. The new app process
and mapped window were verified, the existing account reconnected, and the
installed daemon returned saved previews over IPC. Visual inspection used the
private synthetic fixture. No live send or authentication flow was attempted.

## Inline images, contact pictures, and custom chrome

The app now shows locally staged image attachments in the conversation and
opens them in a native image viewer with fit, zoom, panning, Escape, and an
external-viewer action. Thumbnails use a stable bounded layout and are decoded
when visible. The main title bars use custom GTK window handles and controls.
Fallback avatars use neutral styling. This replaces the earlier GNOME-oriented
visual direction while retaining native GTK behavior.

Contact photos use the existing `contacts.list` IPC response. Matching uses
exact normalized phone numbers or email addresses, with no name-only or suffix
matching. Conflicting photos for an address remain unresolved. Contact snapshots
are correlated to their request; removed or replaced photo bindings are cleared,
and old in-flight photo decodes cannot repopulate a newer photo cache.

The image loader has two workers, a 32-job queue, a 64 MiB texture-cache budget,
and 96 retained decoded cache entries. Visible widgets may also retain their displayed
texture. Off-screen widgets release their images. Input files must be regular,
non-symlink files no larger than 25 MiB. The decoder rejects source dimensions
over 32,768 pixels on either edge or 100 megapixels in total and bounds decoded
output size. Images remain local. Unsupported or missing images show a fallback.

The GUI fixture generates its own PNG attachment and JPEG contact picture.
It verifies actual decoding, the matched header photo, opening the image viewer,
zoom and fit, and the existing draft, search, reconnect, uncertain-send, and
single-instance paths. The custom layout passed dark and narrow light checks.
Formatting and strict workspace Clippy passed. The Rust workspace suite passed
590 tests, with seven environment-dependent tests ignored by default. The GUI
scenario was invoked explicitly through its private-display launcher.

The running daemon had 22 photos in its synced address book, with exact address
matches for 34 direct conversation records. This is metadata evidence; the
fixture supplies the automated rendering evidence. No phone sync, live send,
remote profile-photo request, or authentication flow was performed.

The host-built media app was installed and reopened after its normal draft-saving
window close. The installed binary matches the validated release build. Its
process and mapped window were verified. The daemon process and systemd start
timestamp stayed unchanged during this app-only update. Physical-phone behavior
and live media delivery were not exercised; rendering and viewer behavior were
validated with the isolated generated fixtures.
