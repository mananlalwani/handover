# Known limitations

Handover is pre-1.0. This list is for people inspecting the public tree, not a
promise of completeness.

## Product

- Native phone media volume is not transported. Linux volume from the phone is.
- Overnight idle, twenty laptop suspend cycles, and multi-day soak were not
  waited out on the Milestone 1 device pass. Reconnect after daemon restart,
  app force-stop, phone reboot, and network loss was exercised instead.
- Incoming call actions were not placed against a live ring. Call state was
  observed idle with `place` advertised.
- Samsung has no `cmd clipboard`. Phone-to-Linux clipboard is the in-app send
  action, not a scripted clip inject.
- mDNS discovery has unit tests. Manual `address:port` (including Tailscale)
  is the path that has been live-tested most.
- Some networks need USB `adb reverse` to reach a laptop that is not on the
  same L3 path as the phone.
- The Quickshell example is a reference client. It is not a 1.0 desktop UI.
- Unix-socket IPC is protocol 1 in this tree and is still experimental for
  outside clients.

## Packaging and CI

- Install is from source (`make install-user`). There is no versioned distro
  package yet.
- Go format, vet, tests, race, and vuln scans belong to
  `handover-gmessages`, not this repository.
- The cross-repo Google Messages fixture job is gated on
  `HANDOVER_GMESSAGES_PUBLIC` because the sibling repo may stay private until
  both are public.
- `make systemd-smoke` exists locally. It is not a CI job yet.
- Align Rust and Go rules for state-directory validation and permissions.
- Keep helper-contract fixtures in sync with `handover-gmessages`.
- Snapshot chunking should cover every collection, not only notifications.

## Code size still worth splitting later

These files are large because they own one domain or their tests, not because
the public API is mixed:

- `crates/handover-gmessages/src/messaging.rs` (much of it tests)
- `crates/handover-native/src/lib.rs` (much of it tests)
- `crates/handover-native/src/session/connection.rs` (one session loop)
- `quickshell/HandoverService.qml`
- `quickshell/pages/MessagesPage.qml`

Track further splits as ordinary issues. Do not block public inspection on them.

## License boundary

`crates/handover-gmessages` holds the helper contract, normalization, staging,
and supervision. It does not contain Google protocol sources. If you find
copied `libgm` or `gmproto` material here, that is a bug.
