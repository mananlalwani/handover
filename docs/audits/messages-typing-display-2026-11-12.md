# Typing display parity

QML displays incoming typing alongside the conversation title. The Rust app had
moved it to a small, muted label above the composer. The new header-placement
assertion failed against that layout before the change.

The Rust app now shows a blue Typing label beneath the conversation title and
in the corresponding chat-list preview. It replaces the header transport label
while active. The short visible label remains readable in narrow windows;
tooltips and accessible labels retain the participant names. Normal labels
return after an empty typing update or disconnection.

Sidebar labels update in place without rebuilding rows or changing selection.
Typing in another conversation changes only that conversation's preview. No
backend, protocol, or capability semantics changed. The app does not manufacture
typing events or infer them from message arrivals.

The private GUI scenario checks header placement, mapped visibility, readable
text at 760 pixels, sidebar updates, restoration, and typing in another chat.
Light and dark screenshots use synthetic contacts and messages. These checks
verify UI behavior; they do not establish live Google RCS event delivery or
resolve the previously observed RCS registration rejection.

Formatting, strict workspace Clippy, 626 workspace tests with 7 ignored, Android
assembly/lint/unit tests, QML lint, and 49 JavaScript tests passed. The GUI scenario
passed at the regular light layout and at 760 pixels in dark mode. The copy-menu
test now waits for its popdown to finish before reopening it. Installed the
matching release app and reopened it through normal window closure. The daemon
and native helper were left running. No live typing events were fabricated, and
no messages were sent.

## Composer typing and reconnect follow-up

Typing also appears immediately above the message box with participant names.
The composer reserves the label's height so typing does not move the text box.
Header and sidebar indicators remain in place. The private GUI scenario checks
that all three indicators respond to the same event and clear together.

The main menu now has Reconnect. It uses the existing `messages.sync` contract
for the selected conversation's account, the account filter, or all known
accounts. The helper can restore a stopped receive worker using the saved
pairing. The action never unpairs the phone or replays outgoing operations.
A GUI fixture verifies the exact scoped IPC request and confirms that acceptance
leaves an offline account offline until an authoritative account update arrives.
Close Google Messages browser tabs before using it to avoid competing receivers.

The follow-up passed formatting, strict workspace Clippy, 626 workspace tests
with 7 ignored, Android assembly/lint/unit tests, QML lint, 49 JavaScript tests,
and private GUI checks at 1120 pixels in light mode and 760 pixels in dark mode.
Screenshots contain synthetic data. Live browser-to-native receive recovery and
live typing delivery were not tested in this follow-up; the separate Google RCS
registration rejection remains unresolved.

Installed the matching release binary atomically. The running app did not finish
normal closure, so its window was preserved. The update loads on its next launch.
The daemon and helper were left running. No messages were sent.
