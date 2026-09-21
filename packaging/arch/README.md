# Arch packages

These PKGBUILDs are for a user systemd install under `/usr`. They are not on
the AUR until you push them to `aur.archlinux.org`.

## This machine

From a clone of this repo:

```sh
cd packaging/arch/handover-git
makepkg -si
systemctl --user daemon-reload
systemctl --user enable --now handoverd.service
```

`handover-git` tracks GitHub `main`. `handover` builds the `v0.3.1` source
tarball. The two conflict. Use `-git` while you are developing.

Pair the phone with `handoverctl native pending` as in the user guide. The
Quickshell example is `/usr/share/handover/quickshell/example.qml`.

Disable a previous `make install-user` copy first so you do not run two
daemons:

```sh
systemctl --user disable --now handoverd.service
rm -f ~/.local/bin/handoverd ~/.local/bin/handoverctl
rm -f ~/.local/share/systemd/user/handoverd.service
```

## AUR

```sh
git clone ssh://aur@aur.archlinux.org/handover-git.git
cp PKGBUILD .SRCINFO  # generate with makepkg --printsrcinfo
```

Keep Google Messages in the separate `handover-gmessages` package. That
repository is AGPL-3.0-only.
