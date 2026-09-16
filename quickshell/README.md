# Quickshell reference client

This directory contains a small Quickshell client for Handover protocol 1. It
connects directly to `$XDG_RUNTIME_DIR/handover/handoverd.sock`; it does not run
`handoverctl` or communicate with KDE Connect.

`HandoverService.qml` is a singleton with these useful properties:

- `devices`: the current normalized device array
- `notifications`: current active phone notifications
- `connected`: whether the daemon socket is connected
- `lastError`: the most recent connection or protocol error

`dismiss(notification)`, `invokeAction(notification, action)`, and
`reply(notification, text)` send commands over the socket. `commandFinished`
reports whether the daemon accepted each command; `lastError` shows failures.
The KDE Connect backend currently exposes no action list over D-Bus, so
`invokeAction` is ready for advertised actions but there are no action buttons
for current KDE Connect notifications.

The service requests a snapshot and subscribes after every connection. If the
daemon is absent or restarts, it clears its local view and retries after two
seconds.

With `handoverd` running, launch the minimal example from the repository root:

```sh
quickshell --path quickshell/example.qml
```

The window displays the first connected device as `Name · 69%` and one phone
notification card. Repliable notifications have a text field and clearable
notifications have a Dismiss button. The card is intentionally not a full
notification center and does not take over Quickshell's `NotificationServer`.
