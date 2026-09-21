# Known limitations

Handover is pre-1.0. These are the known product gaps and limits of current
testing.

## Product

- Native media controls do not include phone volume. The phone can control
  Linux volume.
- The Milestone 1 device pass did not cover overnight idle, twenty laptop
  suspend cycles, or multiple days of continuous use. It did cover reconnects
  after daemon restart, app force-stop, phone reboot, and network loss.
- Incoming call actions have not been tested during a live incoming call.
  The device pass observed idle call state with `place` advertised.
- The tested Samsung device has no `cmd clipboard`. Phone-to-Linux clipboard
  testing used the in-app send action.
- mDNS discovery has unit tests. Most live testing used manual `address:port`
  entry, including Tailscale.
- Some networks need USB `adb reverse` to reach a laptop that is not on the
  reachable network as the phone.
- The Quickshell example is a reference client. It is not a 1.0 desktop UI.
- Unix-socket IPC is protocol 1 in this tree and is still experimental for
  outside clients.

## Packaging and CI

- Release tarballs, source builds with `make install-user`, and Arch PKGBUILDs
  under `packaging/arch/` are available. The Arch packages are not published
  to the AUR.
- Go formatting, vet, tests, race detection, and vulnerability checks belong
  to the separate `handover-gmessages` repository.
- The Google Messages fixture comparison runs only when the repository variable
  `HANDOVER_GMESSAGES_PUBLIC` is `true` and Actions can check out the adapter.
- `make systemd-smoke` exists locally. It is not a CI job yet.
- Rust and Go state-directory validation and permission rules still need
  alignment.
- Helper-contract changes require synchronized fixtures in both repositories.
- Large snapshots use chunks for devices, notifications, media, and messaging
  collections. A single oversized record can still exceed the IPC line limit.

## License boundary

`crates/handover-gmessages` holds the helper contract, normalization, staging,
and supervision. It does not contain Google protocol sources. If you find
copied `libgm` or `gmproto` material here, that is a bug.
