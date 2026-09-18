# Handover

Handover connects an Android phone to a Linux desktop. It lets desktop apps
see the phone and use capabilities such as notifications, media controls, calls,
file sharing, and clipboard sync.

Handover runs in the background. You can use its small command-line tool to
check connected phones, or use the included Quickshell panel.

## Current status

Handover is early software. KDE Connect currently provides the widest set of
capabilities. A separate native Android connection is available for testing.

## Install

Handover currently installs from source. On a Linux machine with Rust and KDE
Connect installed:

```sh
git clone https://github.com/mananlalwani/handover.git
cd handover
make install-user
```

Pair the phone with KDE Connect, then check the connection:

```sh
handoverctl devices
```

The full [user guide](docs/user-guide.md) covers pairing, the optional native
Android connection, common commands, troubleshooting, and removal.

## What Handover does

- Shows connected Android phones to Linux desktop apps.
- Receives Android notifications.
- Controls media playing on the phone.
- Supports phone-call actions where the phone and backend allow them.
- Sends files and links to a phone.
- Provides a Quickshell example panel.

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
