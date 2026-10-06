# Arch packages

These PKGBUILDs install Handover under `/usr` and register a systemd user
service. Build either package from its directory with `makepkg -si`.

## Switch from a user installation

If you previously ran `make install-user`, remove its binaries and service
before installing the Arch package:

```sh
systemctl --user disable --now handoverd.service
rm -f ~/.local/bin/handoverd ~/.local/bin/handoverctl
rm -f ~/.local/bin/handover-google-messages-helper ~/.local/bin/handover-google-messages-auth-probe
rm -f ~/.local/share/systemd/user/handoverd.service
systemctl --user daemon-reload
```

## Install

From a clone of this repo:

```sh
cd packaging/arch/handover-git
makepkg -si
systemctl --user daemon-reload
systemctl --user enable --now handoverd.service
```

`handover-git` builds the current Git checkout from GitHub. `handover` builds
the `v0.3.4` source tarball. They provide and conflict with the same package,
so use `handover-git` during development and remove the other package first.

Follow the [user guide](../../docs/user-guide.md#native-android-connection)
to pair the phone. The
Quickshell example is `/usr/share/handover/quickshell/example.qml`.

The package files live in [`handover`](handover) and
[`handover-git`](handover-git). Current source builds include the native MIT
Google Messages helper. The published `v0.3.4` archive predates it; that stable
package retains its existing build until a newer source release is selected.
The separate `handover-gmessages` package is an optional legacy compatibility
backend and remains AGPL-3.0-only.
