# Sharing, clipboard, and desktop helpers

## send-url

```
handoverctl send-url DEVICE URL
```

Sends a URL to one paired device. The share is accepted and, when the
backend supports it, tracked with a transfer id. A printed transfer id
means the request was accepted, not delivered. Watch `monitor` for the
receiver result.

## send-file

```
handoverctl send-file DEVICE PATH
```

Sends one local file to a paired device. The path is canonicalized
locally and shared as a file URL. Accepted, not delivered. Watch
`monitor` for the receiver result.

## notify

```
handoverctl notify DEVICE APP TITLE BODY
```

Sends a notification to one native phone. Queued on the native backend.
Delivery is not confirmed by this command. Use `monitor` to observe the
Android result.

## clipboard

```
handoverctl clipboard DEVICE [TEXT]
```

Sets the clipboard on one native phone. With TEXT, sends that text.
Without TEXT, reads the current Wayland clipboard and sends it. Native
clipboard text is limited to 32 KiB and is not stored in daemon state.

## clipboard-mirror

```
handoverctl clipboard-mirror on
handoverctl clipboard-mirror off
handoverctl clipboard-mirror status
```

Gets or sets Linux-to-phone background clipboard mirroring. Off by
default. `on` enables text-only background mirroring, `off` disables it,
`status` prints the current state. Any other action exits 1.

## clipboard-history

```
handoverctl clipboard-history list
handoverctl clipboard-history save TEXT
handoverctl clipboard-history pin ID
handoverctl clipboard-history unpin ID
handoverctl clipboard-history copy ID
handoverctl clipboard-history clear [--all]
```

Manages phone-to-Linux clipboard history. `list` prints recent and
pinned entries as text previews. `save` stores a new pinned string.
`pin` and `unpin` change the pinned flag of an entry by id. `copy`
copies an entry into the Linux clipboard. `clear` removes recent
entries and keeps pins. With `--all`, `clear` removes pinned entries
as well. IDs come from `clipboard-history list`.

## cancel-share

```
handoverctl cancel-share DEVICE TRANSFER_ID
```

Cancels a queued native share that has not started streaming. Prints
whether the share was cancelled or was already sent or unknown.
Cancellation only works before streaming starts. TRANSFER_ID is the id
printed by the accepting send command.

## screensaver

```
handoverctl screensaver inhibit
handoverctl screensaver release
handoverctl screensaver follow
```

Controls the desktop idle inhibitor. `inhibit` forces the inhibitor on,
`release` forces it off, `follow` returns to the automatic policy, which
inhibits while a native phone is connected or a keep-awake request is
held. Prints the resulting override. Any other action exits 1.

## custom

```
handoverctl custom list
handoverctl custom run NAME
```

Lists or runs allowlisted desktop commands. Only commands in the daemon
allowlist can run. Each runs with fixed arguments, no shell, and no
caller-supplied input. `run` prints whether the command was accepted,
its exit code, and any failure detail when the backend reports it.
