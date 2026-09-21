# Quickshell reference client

This directory contains a small Quickshell client for Handover protocol 1. It
connects to `$XDG_RUNTIME_DIR/handover/handoverd.sock`. It does not run
`handoverctl` and it does not talk to KDE Connect.

## Run the client

With `handoverd` running, launch [`example.qml`](example.qml) from the
repository root:

```sh
quickshell --path quickshell/example.qml
```

The sidebar provides views for Overview, Messages, Calls, Notifications, Media,
Contacts, Clipboard, Phone files, Commands, Remote input, and Transfers.
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

The service requests a snapshot and subscribes after every connection. If the
daemon is absent or restarts, it clears its local view and retries after two
seconds.

## Notifications and sharing

`dismiss(notification)`, `invokeAction(notification, action)`, and
`reply(notification, text)` send commands over the socket. `commandFinished`
reports whether the daemon accepted each command. `lastError` shows failures.
The KDE Connect backend currently exposes no action list over D-Bus, so
`invokeAction` is ready for advertised actions but current KDE Connect
notifications have no action buttons.

`sendFile(device, fileUrl)` and `sendUrl(device, url)` address one device from
`devices`. `shareFinished` reports whether the daemon accepted the request, not
delivery. The example includes a one-file chooser and a URL field. Incoming
share events carry the source device. On the KDE Connect backend, the file or
URL is already handled before that signal reaches Handover.

## Media

`mediaCommand(session, action, value)` sends a supported media control such as
`play`, `pause`, `play_pause`, `previous`, `next`, `seek`, or `set_position`.
The daemon validates the command. Playback state updates only when the backend
reports it. `mediaFinished` reports whether a command was accepted. The
example includes a small media card with previous, play/pause, and next,
gated by each session's advertised capabilities. When seeking is supported,
the card offers ten-second back and forward requests. It does not animate a
progress bar from sparse backend updates.

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
toggles, typing display, read state, and conversation creation. Pairing stays
in `handoverctl messages login`; see the [messaging command
reference](../docs/cli/messages.md). The UI only shows the helper's
verification prompt.

`refreshContacts()` reads the contacts snapshot held by the daemon.
`syncContacts(device)` asks one connected native phone for a fresh snapshot.
Android must grant Contacts access first. The Messages view uses matching
phone numbers and email addresses from that snapshot for conversation and
sender names.
