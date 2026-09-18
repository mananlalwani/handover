# Handover

Handover connects an Android phone to a Linux desktop. It lets desktop apps
see the phone and use capabilities such as notifications, media controls, calls,
and file sharing.

Handover runs in the background. You can use its small command-line tool to
check connected phones, or use the included Quickshell panel.

## Current status

Handover is early software. The native Handover Android connection is the main
path and is available for testing. KDE Connect is an optional compatibility
backend for capabilities that the native connection does not provide yet.

## Install

Handover currently installs from source. On a Linux machine with Rust installed:

```sh
git clone https://github.com/mananlalwani/handover.git
cd handover
make install-user
```

Build and install the Android companion, enable its connection, and pair it
with the Linux daemon. Then check the connection:

```sh
handoverctl devices
```

The full [user guide](docs/user-guide.md) covers native pairing, the optional
KDE Connect backend, common commands, troubleshooting, and removal.

## What Handover does

- Shows connected Android phones to Linux desktop apps.
- Receives Android notifications.
- Controls media playing on the phone.
- Supports phone-call actions where the phone and backend allow them.
- Sends files and links to a phone.
- Sends explicit desktop notifications to a native phone with `handoverctl notify`.
- Can ring or ping a connected native phone.
- Reports the phone's active network transport.
- Inhibits idle and sleep while a native phone is connected.
- Pauses local MPRIS players when a phone call becomes active, when `playerctl`
  is installed.
- Provides a Quickshell example panel.

Native clipboard transfer is explicit. Use the phone's "Send current clipboard
to Linux" action, or `handoverctl clipboard <device> [text]` to set the phone's
clipboard. With no text argument, the CLI reads the current Wayland clipboard.
Clipboard contents are bounded and are not mirrored in daemon state.

Clipboard synchronization is currently provided by the optional KDE Connect
backend. It is not a native Handover capability yet.

The available capabilities depend on the connection method and Android permissions.
Handover reports when a request was accepted. That does not always mean the
phone has completed it.

## Privacy

Native connections use TLS and require pairing. Private keys stay in Android's
keystore. Handover does not log message text, notification
contents, tokens, keys, or file contents.

## More information

- [User guide](docs/user-guide.md)
- [Contributing](CONTRIBUTING.md), for developers
- [MIT license](LICENSE)

The optional Google Messages integration is maintained in a separate repository:
[handover-gmessages](https://github.com/mananlalwani/handover-gmessages).
