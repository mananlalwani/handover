# Quickshell reference client

This directory contains a small Quickshell client for Handover protocol 1. It
connects to `$XDG_RUNTIME_DIR/handover/handoverd.sock`. It does not run
`handoverctl`.

## Run the client

With `handoverd` running, launch [`example.qml`](example.qml) from the
repository root:

```sh
quickshell --path quickshell/example.qml
```

Use `handover-gui --send` to open the dedicated send picker. A file manager can
open a selected file with `handover-gui --send <file-or-URL>`; the
installed **Send to phone** desktop entry is registered for generic files and
as an optional HTTP/HTTPS handler. Desktop and browser chooser support varies.
The picker accepts one file or an HTTP/HTTPS link and asks you to choose a
connected device. It distinguishes daemon acceptance, sender progress, and
transfer completion. **Cancel queued transfer** asks the daemon to remove a share
still in its local send queue. Once the backend takes it for delivery, it cannot
be recalled; a rejected cancellation leaves the transfer waiting for its actual
result. Closing the window or choosing New share does not cancel an accepted
transfer. After a daemon disconnect, the outcome remains unknown.
The launcher passes the optional resource through `HANDOVER_SHARE_RESOURCE`.

The sidebar groups conversations and playback under Workspace, sharing under
Sharing, and less frequent actions under Tools. Overview focuses on devices;
media, notifications, and transfers each have their own page. The device picker
at the top selects the phone used for clipboard, file browsing, calls, contact
sync, and remote input. Device selection stays stable when device lists reorder.
Overview lists native phones with ping, ring, lock, keep-awake, and tethering
actions. Clipboard includes history, and Commands runs names from the local
allowlist.

## State and connection

[`HandoverService.qml`](HandoverService.qml) is a singleton. Its main
properties are:

- `devices`: current normalized device array
- `notifications`: current active phone notifications
- `mediaSessions`: current remote media sessions, including metadata, playback
  state, and supported controls
- `contacts`: latest on-demand native contacts snapshot
- `connected`: whether the daemon socket is connected
- `lastError`: most recent connection or protocol error
- `lastReceivedShare`: most recent transient incoming file or URL event in
  this shell instance

The subscription socket carries the handshake, snapshots, and live events.
Ordinary queries and commands each use a separate, unsubscribed connection with
one request. Replies, errors, and chunk assembly belong to that request. A
background history or settings failure cannot finish a pending send or media
command. The archive-size dialog also waits for its own settings result.

At most 16 requests run concurrently. Additional requests are rejected locally,
with no queue of mutations. A newer collection refresh can supersede an older
read of the same collection; paged history and mutations are not superseded.
`sendRequest` returning true means the client admitted the request, not that the
daemon accepted or completed it. Connecting has a 15-second deadline; after
submission, a reply has up to 255 seconds to allow the helper's four-minute limit.

Losing a command's reply reports an unknown outcome and never resends it. Losing
the subscription aborts outstanding requests, clears disposable views, and
reconnects after two seconds. New snapshots rebuild state. Conversation and
outgoing-operation state come from the ordered subscription rather than extra
startup queries. Physical-device effects still require backend evidence.

The optional `handover-desktop` service mirrors phone notifications through
the Linux desktop notification service, without Quickshell. Enable it with
`systemctl --user enable --now handover-desktop.service`. Desktop actions route
to the phone, and a Dismiss action appears only when the phone marks a
notification clearable. Closing or expiring a desktop popup does not dismiss
the phone notification. The generic notification specification does not
provide inline replies for this client.

## Notifications and sharing

`dismiss(notification)`, `invokeAction(notification, action)`, and
`reply(notification, text)` send commands on isolated connections.
`commandFinished` reports whether acceptance was observed. A connection error
after submission leaves an unknown outcome, described in its error string.
`lastError` shows the most recent error, not the status of every pending command.

`sendFile(device, fileUrl)` and `sendUrl(device, url)` address one device from
`devices`. `shareFinished` reports whether daemon acceptance was observed, not
delivery. Its final `unknown` flag distinguishes a lost reply from a reported
rejection. `shareCancelFinished` has the same flag for an uncertain cancellation;
the original transfer can still report a result. The Send & transfers page includes a file chooser, a link field, and queued-transfer
cancellation. It tracks acceptance, progress, completion, and unknown outcomes
using the same state handling as the dedicated send picker. Incoming
share events carry the source device.

## Media

`mediaCommand(session, action, value)` sends a supported media control such as
`play`, `pause`, `play_pause`, `previous`, `next`, `seek`, or `set_position`.
The daemon validates the command. Playback state updates only when the backend
reports it. `mediaFinished` reports whether a command was accepted. The
Media page separates laptop audio streaming from phone playback, with
previous, play/pause, and next gated by each session's advertised capabilities.
It does not animate a progress bar from sparse backend updates.

## Messages and contacts

Messaging state is account-scoped, never device-scoped:
`messagingAccounts`, `conversations`, `conversationMessages` (keyed by
`account:thread`), `typingStates`, `readStates`, and `pairingPrompt`.
`refreshMessaging`, `loadConversations`, `loadHistory`, `sendText`,
`sendAttachment`, `react`, `unreact`, `markRead`, `sendTyping`,
`deleteMessage`, `openConversation`, `syncAccount`, and `logoutAccount` speak
the additive protocol-1 `messages.*` methods.
`messagingFinished`/`messagingAccepted` report acceptance, never delivery. The
example Messages card has an account picker, conversation list with unread
counts, paged history, compose with attachments, reply targeting, reaction
toggles, typing display, read state, and conversation creation where supported.
Account setup uses the Messages view's **Connect** action and an installed
Chromium browser. The view retains the phone verification symbol until pairing
finishes. **Disconnect** asks the native provider to unpair the phone before
removing saved credentials. Commands remain capability-dependent; see the
[messaging command reference](../docs/cli/messages.md).

`refreshContacts()` reads the contacts snapshot held by the daemon.
`syncContacts(device)` asks one connected native phone for a fresh snapshot.
Android must grant Contacts access first. The Messages view uses matching
phone numbers and email addresses from that snapshot for conversation and
sender names.
