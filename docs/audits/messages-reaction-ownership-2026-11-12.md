# Reaction removal ownership

The message menu created a "Remove my" button for every reaction aggregate.
It never checked actor identities. The private GUI regression started with a
peer's thumbs-up and failed because it exposed that removal action.

The menu now requires an exact match between a reported reaction participant ID
and a conversation participant explicitly marked self. Counts, message direction,
emoji choice, and identity prefixes do not establish ownership. Missing or
unresolved actor identity hides removal. The handler repeats the check against
the current message before submitting, blocking stale menu callbacks after an
ownership update, deletion, or removal of the message.

A GUI scenario verifies that the peer's reaction stays visible, reaction command
acceptance does not fabricate ownership, a reported mixed self/peer aggregate
allows removal of the self reaction, and a stale button cannot submit after the
self actor disappears. Counts wait for backend updates. Model tests cover peer,
self, mixed, unknown and absent actors, another account with the same local
conversation ID, deleted messages, and missing messages.

This changes only the Rust UI. It does not infer or rewrite native Google actor
identities. If those IDs cannot be matched to the normalized self participant,
removal remains unavailable until the backend supplies that evidence. Public
models, IPC, daemon validation, and phone protocol behavior are unchanged.

The GUI scenario's history section now waits for the duplicate update's actual
text and viewport restoration before clicking Latest. The old wait matched the
already-present original arrival, allowing an unrelated render to interrupt the
subsequent action. The original new-message-count and bottom-position assertions
remain intact.

Formatting, strict workspace Clippy, 634 workspace tests with 7 ignored, Android
assembly/lint/unit tests, QML lint, 49 JavaScript tests, and final regular light
and narrow dark GUI scenarios passed. The release app was installed and reopened
through normal window closure, preserving drafts. Its running executable matched
the release build. The daemon and helper were left running. No live reaction
commands were sent; ownership behavior was verified with synthetic GUI events
and the private daemon fixture.
