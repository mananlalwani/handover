# Handover

Handover connects your Android phone to your Linux desktop. Share files, links,
and clipboard contents, read phone notifications, control media playback, and
access calls and contacts from Linux. The Android app also provides presentation,
pointer, keyboard, and Linux volume controls.

`handoverd` runs in the background. Use `handoverctl` from the terminal or the
included Quickshell reference client. The native Android connection works
without KDE Connect, which remains an optional compatibility backend.

Handover is pre-1.0. Capabilities depend on Android permissions and device
support, and the IPC contract is still experimental. See the
[known limitations](docs/KNOWN_LIMITATIONS.md) for gaps and live-testing coverage.

## Install

### Linux

Download the Linux tarball from
[Releases](https://github.com/mananlalwani/handover/releases), unpack it, and run
`./install.sh` from the extracted directory. Release builds target x86_64 Linux
with glibc and are built on Ubuntu 24.04.

The installer copies the binaries to `~/.local/bin` and starts `handoverd` as a
systemd user service. Add `~/.local/bin` to your shell's `PATH`, then check it:

```sh
systemctl --user status handoverd
handoverctl devices
```

To build from source, install Rust and a C linker, then run:

```sh
git clone https://github.com/mananlalwani/handover.git
cd handover
make install-user
```

On Arch Linux, you can build a system package from the clone instead:

```sh
cd packaging/arch/handover-git
makepkg -si
systemctl --user enable --now handoverd.service
```

If you previously used `make install-user`, follow the
[Arch installation guide](packaging/arch/README.md) to switch installations.

### Android and pairing

Install the Android debug APK from the same release, or follow the
[build instructions](CONTRIBUTING.md) to build it yourself. Open Handover on the
phone and enable the connection while the phone and computer can reach each
other.

On Linux, list pending pairing requests:

```sh
handoverctl native pending
```

Compare the eight-digit code shown on Linux with the code on the phone. Only
approve the request if they match, substituting the pending ID and code below:

```sh
handoverctl native pair <pending-id> <eight-digit-code>
handoverctl devices
```

If discovery does not find the computer, enter its address and port `24837` in
the Android app. A reachable Tailscale address works too. Grant the Android
permissions for the capabilities you want to use.

### Optional desktop interface

With Quickshell installed, launch the reference client:

```sh
quickshell --path ~/.local/share/handover/quickshell/example.qml
```

For an Arch package installation, use
`/usr/share/handover/quickshell/example.qml`. See the
[Quickshell guide](quickshell/README.md) for the available views and integration
API.

## Everyday use

Replace `"Phone"` with the device name or ID from `handoverctl devices`.

```sh
handoverctl devices
handoverctl notifications
handoverctl media
handoverctl send-url "Phone" https://example.com
handoverctl send-file "Phone" ./photo.jpg
handoverctl clipboard "Phone" "Text to copy to the phone"
handoverctl notify "Phone" app title body
handoverctl monitor
```

To send the current Wayland clipboard, run `handoverctl clipboard "Phone"`
without a text argument. For the other direction, use "Send current clipboard
to Linux" in the Android app. Background clipboard mirroring is an opt-in
Android setting and is off by default.

A successful command means the backend accepted the request. Completion and
state updates appear in `handoverctl monitor` when the backend provides them.

The [user guide](docs/user-guide.md) covers setup, permissions, troubleshooting,
and removal. The [CLI reference](docs/cli/handoverctl.md) is also available as
`man handoverctl` after installation.

## Optional integrations

[Handover Google Messages](https://github.com/mananlalwani/handover-gmessages)
is a separate install for SMS/RCS on Linux. The adapter is AGPL-3.0-only and runs
as a separate process. Google credentials stay with the adapter.

KDE Connect can remain paired for compatibility with its existing capabilities.

## Development and support

See [Contributing](CONTRIBUTING.md) for build and test instructions,
[Design](DESIGN.md) for the architecture, and the [Roadmap](ROADMAP.md) and
[Vision](VISION.md) for planned work.

Report bugs through [GitHub Issues](https://github.com/mananlalwani/handover/issues).
For security vulnerabilities, follow [SECURITY.md](SECURITY.md) and use private
reporting.

This repository is licensed under [MIT](LICENSE).
