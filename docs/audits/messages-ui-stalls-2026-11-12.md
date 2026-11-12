# Messages UI stalls

The installed app was idle during sampling, so the desktop's intermittent
"not responding" popup was not reproduced live. Its daemon snapshot contained
426 conversations across accounts. Inspection found that every subscription
event rebuilt the entire GTK chat list, including unrelated media updates.
Conversation updates for other chats also redrew the open message thread.

A private GUI regression sent an unrelated media event and asserted that the
selected row and message widgets survived. It failed against the previous code
because the selected row was replaced. After the fix, unrelated events return
without rendering. Messaging events redraw the open thread only when they affect
its conversation. Chat-list updates from events and startup replies coalesce
through one 16 ms timer. Search results still render immediately.

The GTK receive task yields for 1 ms after spending at least 8 ms processing
updates. This lets the main loop handle input, frames, and window responsiveness
checks even when the bounded event channel remains ready. Individual handlers
still run synchronously; this does not establish an upper bound for every frame.
No protocol, backend state, pairing, or send behavior changed.

The GUI scenario also adds 426 synthetic conversations, applies 100 inventory
updates to other chats, and checks that the open thread is untouched, the list
waits for one scheduled update, selection survives, and the final update appears.
Both regular light and 760-pixel dark GUI scenarios passed. Formatting, strict
workspace Clippy, 626 workspace tests with 7 ignored, Android assembly/lint/unit
tests, QML lint, and 49 JavaScript tests passed.

Installed the matching release binary and reopened Messages through normal
window closure. The running executable matched the release build. The daemon
and helper remained running. No messages were sent. The tests verify the
unnecessary rebuild and burst handling; continued live use is still needed to
confirm whether this accounts for every reported popup.
