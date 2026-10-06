# Known limitations

Handover is pre-1.0. Public interfaces may change.

## Devices and desktop

- Native contact sync supports complete generations of up to 4,096 contacts,
  8 MiB, and 256 chunks. Failed or incomplete generations preserve the last
  complete list. Older Android builds can send an unmarked truncated list.
- Native media controls do not include phone volume. The phone can control
  Linux volume.
- Android restricts background clipboard access. Use the companion's in-app
  send action when automatic phone-to-Linux clipboard access is unavailable.
- Network discovery requires multicast reachability. Use manual `address:port`
  entry when discovery is blocked, including across Tailscale networks.
- The Quickshell example is a reference client, not a stable desktop application.
- Unix-socket IPC uses protocol 1 and is experimental for outside clients.
- Large snapshots use bounded chunks. A single oversized record can still exceed
  the IPC line limit.

## Google Messages

- The native client requires Google Messages on the phone and Google's service.
  It is not a direct carrier RCS implementation.
- Capabilities depend on the connected phone and selected helper. The native
  helper does not support creating conversations, deleting messages, sending
  typing notifications, or marking messages read.
- Message history is a bounded cache, not a complete persistent archive.

## Packaging

Release tarballs, source installation, and [Arch PKGBUILDs](../packaging/arch/README.md)
are available. The Arch packages are not published to the AUR. Optional browser
setup requires Python 3 and an installed Chromium browser. Google sign-in
compatibility depends on the browser and Google's authentication policy.
