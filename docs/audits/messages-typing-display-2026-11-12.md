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
