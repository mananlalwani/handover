# Unread dot updates

The Rust app used only `Conversation.unread_count` for sidebar dots. It discarded
read states from the initial subscription and ignored live `ReadState` events.
The private GUI regression sent a reported read event for an unread fixture chat
and failed because its dot remained visible.

The connection now passes initial read states to the UI model. Snapshot recovery
and live events update a conversation-scoped, bounded read-state map. A later
explicit unread count supersedes an older read event; an unknown count preserves
that event. Conversation/account removal clears the corresponding state. Unknown
read status remains unknown and is never converted into a confirmed read.

The GUI scenario verifies the initial snapshot, clearing a dot from a read event,
and restoring it from a later unread event. Model tests cover account scoping,
unknown counts, newer explicit counts, and removal. Provider read receipts on
outgoing messages remain separate from the local user's conversation read state.

Read-only inspection found unknown unread counts for the active native account.
The native helper does not implement mark-read and does not advertise that
capability. This UI fix does not add provider mark-read support or prove that the
user's intermittent live symptom has been identified. No live messages were
marked read and no messages were sent.

Formatting, strict workspace Clippy, 629 workspace tests with 7 ignored, Android
assembly/lint/unit tests, QML lint, 49 JavaScript tests, and regular light/narrow
dark GUI scenarios passed. Installed the matching release and reopened Messages
through normal closure. The running executable matched the release build;
drafts were preserved and the daemon/helper were left running.
