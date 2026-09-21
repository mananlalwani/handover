# Handover

Handover connects an Android phone to a Linux desktop so desktop apps can see
the phone and use notifications, media controls, calls, files, and related
device actions.

It runs in the background. The daemon owns the live state. `handoverctl` and
the included Quickshell panel are clients of that daemon, not a second source
of truth.

This repository is MIT-licensed and pre-1.0. IPC types and methods may still
change. Treat the Unix-socket protocol as experimental until a versioned 1.0
is documented.

## Current status

The native Handover Android connection is the path for daily use. Milestone 1
is closed for that path. KDE Connect is an optional compatibility backend, not
a requirement for the native capabilities in the table below.

Google Messages support is optional and lives in a separate AGPL-3.0-only
repository. It does not ship as part of this MIT tree.

## License

This repository is [MIT](LICENSE).

The optional Google Messages adapter is
[AGPL-3.0-only](https://github.com/mananlalwani/handover-gmessages). Handover
talks to it as a separate process over a coarse JSON helper contract. Google
protocol code, cookies, and tokens stay in that adapter. Do not copy them into
this tree.

## Install

Tagged GitHub Releases attach a Linux tarball (`x86_64-unknown-linux-gnu` from
Ubuntu 24.04) and a debug APK. Unpack the tarball and run `./install.sh`. That
copies binaries to `~/.local/bin`, the user systemd unit, completions, and the
Quickshell example. Put `~/.local/bin` on `PATH`.

Install from source if you have Rust and want to build locally:

```sh
git clone https://github.com/mananlalwani/handover.git
cd handover
make install-user
```

`make dist` builds the same tarball layout without installing it.

Build the Android companion from `android/` with `./gradlew assembleDebug`,
or install the APK from the GitHub Release. Enable the connection, and pair:

```sh
handoverctl native pending
handoverctl native pair <pending-id> <eight-digit-code>
handoverctl devices
```

The [user guide](docs/user-guide.md) covers pairing, commands, troubleshooting,
and removal.

## What Handover does

- Shows connected Android phones to Linux desktop apps.
- Receives Android notifications.
- Controls media playing on the phone.
- Supports phone-call actions where the phone and backend allow them.
- Sends files and links to a phone, and receives shares from the phone.
- Sends explicit desktop notifications to a native phone with `handoverctl notify`.
- Supports an on-demand contacts snapshot.
- Provides a phone presentation remote with slide and pointer controls.
- Provides explicit phone controls for Linux volume up, down, and mute.
- Supports explicit clipboard transfer in both directions.
- Can ring or ping a connected native phone.
- Reports the phone's active network transport and battery.
- Inhibits idle and sleep while a native phone is connected.
- Pauses local MPRIS players when a phone call becomes active, when `playerctl`
  is installed.
- Provides a Quickshell example panel.

Native clipboard transfer is explicit. Use the phone's "Send current clipboard
to Linux" action, or `handoverctl clipboard <device> [text]`. With no text
argument, the CLI reads the current Wayland clipboard. Contents are bounded
and are not mirrored in daemon state. Text, HTML, and URI payloads are limited
to 32 KiB each and 48 KiB together. File-backed clipboard items are limited to
10 MiB. Background clipboard mirroring is an opt-in Android setting while the
foreground service is running.

Handover reports when a request was accepted. That does not always mean the
phone completed it.

## Capability status

| Capability | Status |
| --- | --- |
| Pairing, discovery, reconnect, and persistent identity | NATIVE |
| Notifications, updates, removals, actions, and replies | NATIVE, permission-dependent |
| Media state and playback controls | NATIVE, permission-dependent |
| Phone media volume over the native transport | OUT OF SCOPE |
| Calls and contacts | NATIVE, permission-dependent |
| Clipboard, files, and links | NATIVE |
| Background clipboard mirroring | NATIVE opt-in, or KDE FALLBACK |
| Presentation, pointer, keyboard, and Linux volume controls | NATIVE |
| Battery and network state | NATIVE |
| Remote filesystem browsing | NATIVE, bounded to the user's home or Android shared storage |
| Configurable remote commands | NATIVE, local allowlist only |
| Browser integration | NATIVE through URL sharing and Android share targets |
| VPN and non-LAN operation | NATIVE when a reachable address is entered manually |
| Google Messages conversations | OPTIONAL, separate AGPL adapter |
| First-party Messages, Calls, or Contacts applications | PLANNED |
| Activity handoff and shared drafts | PLANNED |
| First-party GNOME or KDE shells | PLANNED |
| Stable 1.0 IPC for third-party clients | PLANNED |
| iOS | OUT OF SCOPE |
| Independent Google protocol implementation in this repo | OUT OF SCOPE |
| Support for every Linux distro and Android OEM | OUT OF SCOPE |

Known gaps and assumptions are listed in [docs/KNOWN_LIMITATIONS.md](docs/KNOWN_LIMITATIONS.md).
Architecture is in [DESIGN.md](DESIGN.md). Daemon, native transport, IPC,
Android companion, and the Quickshell client are the main seams. Public models
live in `handover-core` and stay backend-independent.

## Privacy

Native connections use TLS and require pairing. Private keys stay in Android's
keystore. Handover does not log message text, notification contents, tokens,
keys, or file contents.

## Bugs and security

Use [GitHub Issues](https://github.com/mananlalwani/handover/issues) for
ordinary bugs. There is a [bug report template](.github/ISSUE_TEMPLATE/bug_report.md).

Do not file security issues in public. Follow [SECURITY.md](SECURITY.md) and
use GitHub private vulnerability reporting.

## More information

- [Vision](VISION.md)
- [Roadmap](ROADMAP.md)
- [Design](DESIGN.md)
- [User guide](docs/user-guide.md)
- [Known limitations](docs/KNOWN_LIMITATIONS.md)
- [Engineering principles](docs/ENGINEERING_PRINCIPLES.md)
- [Contributing](CONTRIBUTING.md)
- [MIT license](LICENSE)

The optional Google Messages integration is
[handover-gmessages](https://github.com/mananlalwani/handover-gmessages).
