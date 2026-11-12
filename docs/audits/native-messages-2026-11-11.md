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


## Automatic older history

Scrolling within one viewport of the top now requests the next older page.
The app preserves the visible bubble's pixel position as messages arrive and
reuses unchanged rows, including their decoded images. The loading indicator
overlays the conversation; a retry action appears only after failure. Latest
messages scrolls to the retained tail or reloads it if deep browsing discarded
newer records from the bounded 300-message view.

History requests carry a view generation so an older reply cannot overwrite a
newer visit to the same conversation. One older request runs per view at a time.
Repeated cursors and three pages without new message IDs stop automatic loading
and offer retry. The app passes daemon cursors unchanged.

Saved and searched context now includes an optional older-history continuation.
The archive derives it from existing saved page metadata. Missing metadata does
not invent a provider cursor or establish that the provider has no older history.
The SDK keeps its existing context method and adds a page-returning method.

The isolated GTK fixture verifies automatic loading, request deduplication,
pixel-position preservation, unchanged row reuse, stopping at the returned end,
returning to the retained latest messages without a fetch, context continuation,
and reopening the same chat during an older request. Archive and daemon IPC
tests cover saved-context cursors; the model test covers the bounded view and
its discarded newer tail. Provider pagination on the live account and physical
phone behavior were not exercised.

Formatting, strict workspace Clippy, and all 593 workspace tests passed, with
seven environment-dependent tests ignored by default. The GTK scenario passed
when invoked explicitly. Android build, lint, and unit checks passed. QML lint
and all 49 JavaScript tests passed.

The matching host-built daemon, app, CLI, and desktop bridge were installed
after backing up the previous binaries. The app closed through its normal
window-close flow before the daemon restarted. Installed binary hashes match
the release artifacts. The reopened app has a mapped window, both services are
active, and the daemon returns accounts and the persistent workspace over IPC.
The existing account reconnected. This confirms startup and IPC health; the
scroll behavior was exercised with the isolated synthetic conversation.


## Conversation interactions and saved media

Quoted replies show the original sender and saved text. The app resolves missing
originals through saved context without replacing the current conversation.
Clicking a quote scrolls to its loaded original or opens that message's saved
context. Cached originals follow update and removal events. Message actions are
available through right-click and keyboard-accessible menus. Copy and reply use
the existing normalized message and draft models.

Reactions use the existing normalized react/unreact commands only when the
conversation advertises support. The app renders aggregates from message
updates and never increments counts after command acceptance. Reaction errors
have separate wording from text and media sends.

File selection now stages one attachment in the composer. Drag-and-drop uses
the same staging path. The preview supports removal and an optional caption;
only Send submits the file. Acceptance clears only the captured selection and
matching caption draft. Failed or uncertain requests retain both. Attachments
cannot be combined with quoted replies. The composer accepts local regular
UTF-8 paths up to 50 MiB, rejects symlinks, and keeps at most 32 conversation
selections. These selections stay in memory, while captions use the existing
daemon draft persistence. Closing with staged files offers keep or discard.

Photos and files in the conversation header opens a browser over the saved
archive, independent of the current loaded message window. The normalized
`messages.media` method scopes every query to one known account/conversation,
excludes deleted and text-only messages, and returns pages of 16 messages.
Offset is bounded at 10000 and responses use the existing chunk limits. The
browser keeps one page, at most 256 attachments, with one outstanding read and
a ten-second deadline. It does not download missing files or claim a complete
provider archive. Image galleries support Previous/Next and arrow keys as well
as the existing fit, zoom, panning, and external-viewer actions.


The private GTK scenario covers quoted sender/text previews, fetching an
original outside the loaded window without replacing it, jumping into that
original's saved context, and quote update/removal events. It exercises the
message menu, clipboard copy, reply selection, reaction and removal requests,
and unchanged reaction counts after acceptance. It opens the saved-media
browser and navigates between generated images. A real GTK drop signal stages
a single fixture file; a multiple-file drop preserves the existing selection
and caption. Removing an attachment keeps its caption, while accepted file
submission carries the caption and clears the captured draft. Existing history,
reconnect, uncertain-send, setup, and single-instance checks remain covered.

Light and narrow dark fixture layouts passed. Formatting, strict workspace
Clippy, and the Rust workspace tests passed with 598 tests and seven ignored
by default. The GTK scenario passed when invoked explicitly. Android build,
lint, and unit checks passed, along with QML lint and all 49 JavaScript tests.
The fixture supplies the interaction evidence. No live send, reaction mutation,
authentication, or physical-phone flow was exercised.


The matching host-built app, daemon, CLI, and desktop bridge were installed
after backing up their previous binaries. The old app closed normally before
the daemon restarted. All installed hashes match the release artifacts. Both
services are active, the reopened app has a mapped window, and the existing
account reconnected. Read-only live IPC returned the persistent workspace and
five saved media-bearing messages for the selected conversation. This verifies
startup and archive integration; sending and interaction semantics were tested
with synthetic conversations.


## Live conversation feedback

The app now reads incoming typing from the initial subscription, recovery
snapshots, and live typing events. Names use exact participant IDs; unknown
participants show Someone. Empty updates, account disconnection, conversation
removal, and reconnects clear transient presence. Typing updates change only
the reserved composer row and do not rebuild the message list or sidebar.
The client stores at most 512 typing entries with 64 distinct peer IDs per entry.

Outgoing bubbles use exact scoped message identities from daemon outgoing
records and message-status events. There is no content, timestamp, or local-read
state matching. Journal-backed outgoing outcomes take precedence when available.
Statuses absent from recovered daemon state remain unknown and have no label.
The direct-event status cache is bounded at 512 entries and cleared on recovered
snapshots. Acceptance remains distinct from delivery.

While browsing older messages, a live incoming message newer than the known
preview can add to a bounded set of new arrivals. History merges, updates,
self messages, other conversations, and older events do not increment it.
At most 300 IDs are retained; overflow uses a generic New messages label.
Following the current tail or opening the newest view clears the indicator.

The GUI test exposed a race where a message redraw invalidated the frame callback
for Latest messages. The app now retains the requested jump across redraws until
the latest layout reaches the tail. The regression clicks Latest while an
incoming update is still queued and checks both scroll position and count reset.

Four model tests cover typing identity/lifecycle, status correlation, arrival
filtering and bounds, and snapshot/removal cleanup. The GTK scenario covers live
typing and clearing, Delivered/Read on the exact outgoing bubble, stale typing
after reconnect, and new arrivals without moving the visible history anchor.
Its routing subprocess uses a separate subscription, so event injection retains
a handle to the tested window's connection. Temporary diagnostic logging was
removed after confirming fixture routing and the scroll race.


Formatting, strict workspace Clippy, and all 602 Rust workspace tests passed,
with seven ignored by default. The GUI scenario passed explicitly in normal
and narrow dark layouts. Android build, lint, and unit checks passed, along
with QML lint and all 49 JavaScript tests.

The app-only release was installed after its normal window-close flow, with a
backup of the old binary. The installed hash matches the release artifact.
The reopened app has a mapped window and the daemon returns the existing
connected account. The daemon PID and start timestamp stayed unchanged.
Presence, delivery labels, and the arrival/jump race were tested with isolated
fixtures. No live send, reaction, phone typing, or delivery flow was exercised.


## Media playback, conversation search, exports, and attachment drafts

Audio and video attachments now use native GTK playback controls. Opening a
conversation does not prepare every decoder. Play loads the selected file, and
at most four players retain loaded media. Hiding or removing a row pauses and
unloads its player. Status-label updates reuse the row and stream. Source files
must be local regular files of at most 50 MiB. The player opens with no-follow
flags and retains that descriptor while GTK reads its local file URI. Codec
errors show a fixed fallback and an external-player action; playback never starts
without the user's Play action.

Message text escapes markup and creates links only for explicit HTTP/HTTPS URLs.
Copy link actions use the parsed visible URLs. Search highlights use Unicode
case folding with original byte boundaries and merge overlapping term matches.
Save as exports a locally staged file through a bounded, no-follow read and an
atomic destination replacement. Missing or unsupported media is not downloaded.
Export tests cover destination replacement and rejection of nonregular sources.

Find in conversation uses the new normalized `messages.search_conversation`
method. Its account and thread scope is enforced in the archive query and checked
again by the SDK. The global search method is unchanged. Find debounces requests,
keeps at most one read in flight and one result page, ignores old generations,
and navigates through pages of 25 saved hits. It shows known result counts with
+ when more remain. Ctrl+F opens Find, Ctrl+K opens global search, and Enter or
Shift+Enter selects the next or previous match.

Attachment selections use an independent normalized attachment-draft model and
revision. The daemon copies selected files into private storage before replying.
Metadata shares the messaging workspace database; file copies live in
`messaging-draft-files`. There are at most 32 active files, 50 MiB each and
256 MiB of saved bytes. Replacements temporarily retain both copies. The daemon
holds a file-store lock and cleans unreferenced staging directories. Files and
directories use modes 0600 and 0700. Cleared selections retain revision records
to reject stale initial writes, bounded at 4096 records.

The app waits for attachment load/save replies before enabling Send. Captions
retain their independent text-draft revisions. Accepted sends clear only the
captured attachment revision. Failed or uncertain saves keep the selection and
show retry/reload controls. Conflicting saves require reloading the stored
selection. Closing waits for saves; a failed save offers keep or discard.
Disabling persistence uses temporary daemon-session storage and the app reports
that scope.

Archive tests cover scoped matches across accounts and conversations. Workspace
tests copy a file, remove the original, reopen the database, and verify the
saved bytes and revision. They also reject symlink and nonlocal sources, retain
old selections on failure, check stale writes after removal, and verify account
cleanup. The private GUI fixture generates audio/video files and exercises the
new playback, search, links, and draft flows. CI test dependencies now include
the GTK media backend, codecs, and FFmpeg fixture generator.

Validation passed with 616 workspace tests and 7 ignored tests, strict workspace
Clippy, formatting, Android debug assembly/lint/unit tests, QML lint, and all 49
JavaScript tests. The GUI scenario passed at the regular window size and at
760 pixels in dark mode. It checks real decoding of generated WAV and VP8 WebM
files, search navigation, link copying, and attachment restoration after deleting
the original file. The private display uses software rendering. Playback unload
clears the media source without queuing a redundant pause before backend teardown.
The native GTK backend retains cached stream flags after clearing its source, so
the unload assertion checks source removal. Ubuntu CI has not run locally.

Installed matching release app, daemon, CLI, and desktop binaries locally after
closing the app normally. The daemon and Messages services are active, with one
mapped app window. Read-only live IPC confirms persistent workspace storage, an
attachment-draft lookup, and 25 scoped search hits with more available. Every
returned hit belongs to the requested conversation. Three accounts are present;
one is connected and authenticated. No real messages, reactions, authentication,
or phone actions were exercised. Save As filesystem behavior is covered by tests;
the system file dialog was not exercised against live attachments.


## Image format compatibility follow-up

The image decoder now chooses installed raster loaders using file contents and
MIME hints instead of a five-format allowlist. Contact photos no longer assume
JPEG. Attachments with absent or generic MIME metadata can select the image UI
by a registered filename extension. This applies to conversation thumbnails,
the viewer, attachment drafts, and Photos and files. Decoding still checks the
bytes; a filename does not bypass validation. No helper protocol types changed.

Ten original synthetic fixtures cover JPEG, PNG, GIF, WebP, BMP, RGB TIFF, ICO,
HEIC, AVIF, and JPEG XL. All ten were required and decoded locally, including
incorrect and generic MIME hints. Tests verify bounded output dimensions.
The private GUI test now uses HEIC attachments, including one without MIME
metadata, and a PNG contact photo. Thumbnail, viewer, gallery, draft restoration,
and the existing messaging scenario pass with those files.

Input limits and nonregular-file rejection remain in place. Loaders that ignore
the requested decode size receive a final scaling pass before texture creation.
HEIF sequences and animations show still previews. SVG remains outside the
raster viewer. Codec availability and encoding variants still matter: the local
Glycin decoder rejected a 2-bit palette TIFF, while the RGB TIFF fixture passed.
The Arch development package now requires libheif; Ubuntu GUI CI installs HEIF
and WebP loaders. Remote CI was not run during this session.

Validation passed with 618 workspace tests and 7 ignored tests, strict workspace
Clippy, formatting, Android debug assembly/lint/unit tests, and QML lint. The
format test ran with every fixture loader required. Installed the matching
release app after normal window closure; its service is active with one mapped
window. The daemon was not restarted. Image compatibility was tested with
synthetic files, not private account attachments. No real messages were sent.


## Contact photo parity and typing diagnosis

The Rust migration omitted QML's validated NANP country-code equivalence and
ASCII-insensitive email matching, and excluded all group photos. Read-only
comparison of cached data found QML photo matches for 38 direct conversations
and 74 groups, while the original Rust lookup found 32 direct matches and no
groups. This was a client regression, not evidence that all missing photos were
absent from the phone.

The app now preserves those address rules. Direct conversations use one
unambiguous photo; groups use up to four distinct member photos in a local
composite. Conflicting real photos remain ambiguous. Photo-less duplicates no
longer hide the only available photo. Two regressions reproduced that duplicate
bug before the fix. Tests cover international and short-code exclusions,
case handling, group member order, self exclusion, composite pixels, and corrupt
photo fallback. The GUI scenario verifies group photos with national-number and
mixed-case email members, alongside the existing contact/header/viewer checks.

Added closed helper diagnostic codes for typing receiver lifecycle. The daemon
journals only these fixed codes. Successful registration and an open receive
response are distinct evidence. Codes carry no arbitrary helper text, account
addresses, credentials, or provider payloads. Contract fixtures and RCS transport
tests cover decoding and publication order. Public IPC typing semantics did not
change.

Formatting, strict workspace Clippy, 626 workspace tests with 7 ignored, Android
assembly/lint/unit tests, QML lint, 49 JavaScript tests, and the private GUI scenario
passed. Matching release app, daemon, and native helper binaries were installed.
The first live diagnostic run reported waiting for settings, a registration
attempt, and registration rejection. It also found a conversation-cursor failure
in receive recovery. Incoming typing is not yet live-verified or resolved.

Subsequent read-only lookup checks found photos for 38 direct conversations and
76 groups using the installed matching/composite rules. These counts establish
available matches, not a visual inspection of every private conversation.
The normal account recovered as connected and authenticated after the bounded
diagnostics. Registration still returned `InvalidArgument`; an observed-version
trial was reverted. See [the typing investigation](../gmessages-typing.md) for
the first-party module provenance and remaining evidence gap. The shared browser
was signed out, and the temporary sign-in tab was closed.
