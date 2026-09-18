# Quickshell reference client

This directory contains a small Quickshell client for Handover protocol 1. It
connects directly to `$XDG_RUNTIME_DIR/handover/handoverd.sock`; it does not run
`handoverctl` or communicate with KDE Connect.

`HandoverService.qml` is a singleton with these useful properties:

- `devices`: the current normalized device array
- `notifications`: current active phone notifications
- `mediaSessions`: current remote media sessions, including metadata, playback
  state, and supported controls
- `contacts`: the latest on-demand native contacts snapshot
- `connected`: whether the daemon socket is connected
- `lastError`: the most recent connection or protocol error
- `lastReceivedShare`: the most recent transient incoming file/URL event in this shell instance

`dismiss(notification)`, `invokeAction(notification, action)`, and
`reply(notification, text)` send commands over the socket. `commandFinished`
reports whether the daemon accepted each command; `lastError` shows failures.
The KDE Connect backend currently exposes no action list over D-Bus, so
`invokeAction` is ready for advertised actions but there are no action buttons
for current KDE Connect notifications.

`sendFile(device, fileUrl)` and `sendUrl(device, url)` address one device from
`devices`. `shareFinished` reports whether KDE Connect accepted the request;
it does not indicate delivery. The example includes a one-file chooser and a
URL field. Incoming share events carry the source device, but KDE Connect
already saved the file or handled the URL before the signal reaches Handover.

`mediaCommand(session, action, value)` sends a supported media control such as
`play`, `pause`, `play_pause`, `previous`, `next`, `seek`, or `set_position`.
The service validates the command on the daemon side; playback
state is updated only when the backend reports it. `mediaFinished` reports
whether a command was accepted. The example includes a small media card with
previous, play/pause, and next controls, gated by each session's advertised
capabilities. When seeking is supported, the card offers ten-second back and
forward requests; it does not animate a progress bar from sparse backend updates.

Messaging state is account-scoped, never device-scoped: `messagingAccounts`,
`conversations`, `conversationMessages` (keyed by `account:thread`),
`typingStates`, `readStates`, and `pairingPrompt`. `refreshMessaging`,
`loadConversations`, `loadHistory`, `sendText`, `sendAttachment`, `react`,
`unreact`, `markRead`, `sendTyping`, `deleteMessage`, `openConversation`,
`syncAccount`, and `logoutAccount` speak the additive protocol-1
`messages.*` methods; `messagingFinished`/`messagingAccepted` report
acceptance, never delivery. The example includes a straightforward Messages
card: account picker, conversation list with unread counts, paged history,
compose with attachments, reply targeting, reaction toggles, typing display,
read state, and conversation creation. Pairing itself stays in
`handoverctl messages login`; the UI only displays the helper's
verification prompt.

The service requests a snapshot and subscribes after every connection. If the
daemon is absent or restarts, it clears its local view and retries after two
seconds.

With `handoverd` running, launch the minimal example from the repository root:

```sh
quickshell --path quickshell/example.qml
```

The window displays the first connected device as `Name · 69%`, a compact media
card, and one phone notification card. Repliable notifications have a text
field and clearable notifications have a Dismiss button. The cards are
intentionally not a full media or notification center and do not take over
Quickshell's `NotificationServer`.

`refreshContacts()` reads the contacts snapshot held by the daemon. `syncContacts(device)` asks
one connected native phone for a fresh snapshot. Android must grant Contacts access first.
The Messages view uses matching phone numbers and email addresses from that snapshot for
conversation and sender names.
