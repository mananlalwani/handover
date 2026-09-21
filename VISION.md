# Handover vision

Pair the phone once. After that, Linux should treat it as part of the system.

Handover is the Android integration layer for Linux, not an Android emulator
and not a mandatory control-panel app. It owns discovery, trust, connection,
device state, and transfers so other software does not reimplement pairing.

## Success

The laptop wakes and the phone is there. Calls, messages, notifications, media,
contacts, clipboard, and files are available to Linux. Network changes, process
restarts, and suspend recover without a new pairing dance.

Apps speak Handover types (`Device`, `Notification`, `Call`, `Conversation`,
`Contact`, `Transfer`, `MediaSession`). They do not speak KDE Connect packets,
Google protocol objects, or Matrix bridge types.

The user rarely opens a Handover-specific window.

## Architecture

```text
                    Android
                       |
        +--------------+--------------+
        |              |              |
  Native Handover   Google         Other
     protocol       Messages      providers
        |              |              |
        +--------------+--------------+
                       |
                       v
                   handoverd
                       |
                Linux-facing IPC
                       |
        +--------------+--------------+
        |              |              |
 first-party apps   shell clients   third-party apps
```

`handoverd` owns identity, presence, trust, and canonical state. Quickshell
already reconnects and rebuilds from that state. Future Messages, Calls, or
Contacts apps do the same. They do not open a second phone session.

## Principles

Local-first. No Handover account for basic phone and laptop use.

Explicit pairing, then silent reconnect.

Native Android is the primary path. Other backends stay replaceable. Their
wire details stay out of clients.

Accepted is not completed. Report the state the backend actually gives you.

Frames, queues, files, history, retries, and concurrency have limits.

If more than one client needs an operation, put it on the daemon.

Put phone actions where Linux already works: shells, launchers, notifications,
file managers, media, other apps. Do not require one giant Handover GUI.

## Communications

`handover-gmessages` is a provider. It does not define the public messaging
model. Google protocol code stays in that process. A carrier-level RCS stack
is a separate problem and is not required for the current goal.

## Done enough

Common Android and Linux workflows no longer make the user manage the device
boundary. Capabilities look like ordinary Linux services. People forget
Handover is running.
