# Known limitations

Handover is pre-1.0. Public interfaces may change.

## Devices and desktop

- Native contact sync supports complete generations of up to 4,096 contacts,
  8 MiB, and 256 chunks. Failed or incomplete generations preserve the last
  complete list. Older Android builds can send an unmarked truncated list.
- Phone volume controls require matching Android and Linux builds. Android can
  restrict ringtone changes under Do Not Disturb and disable volume changes on
  fixed-volume devices. Handover displays the phone's reported levels.
- Desktop call audio requires a paired Bluetooth phone exposing a duplex HFP
  gateway profile, PipeWire, `pactl`, `pw-loopback`, `pw-dump`, and BlueZ. Select the
  Bluetooth endpoint, microphone, and speakers explicitly in Calls. Physical
  call audibility and Bluetooth recovery still need validation with a phone.
- Phone downloads support files up to 8 GiB, folders up to 10,000 entries, and
  16 GiB of reserved partial data. Interrupted downloads need an explicit Resume.
  Linux-to-phone sends and older Android download peers retain the 100 MiB limit.
  Android's protected app data remains inaccessible.
- The optional desktop MPRIS bridge exports sessions with a known playback
  state and the phone's advertised Play, Pause, Play/Pause, Next, and Previous controls.
  Unknown playback sessions are omitted. Position is the latest phone report,
  not a continuously updated estimate. MPRIS seeking is unavailable because
  the normalized model has no track ID and the Android backend does not expose
  relative Seek.
- Android restricts background clipboard access. Use the companion's in-app
  send action when automatic phone-to-Linux clipboard access is unavailable.
- Network discovery requires multicast reachability. Use manual `address:port`
  entry when discovery is blocked, including across Tailscale networks.
- The native [Messages app](messages-app.md) currently covers existing
  conversations, saved-message search, drafts, quoted replies, attachment
  composing, saved photos/files, and capability-gated reactions. Attachment
  selections and captions persist through the daemon unless storage is disabled.
  Inline audio/video playback depends on installed GTK media codecs.
  The media browser lists saved history and cannot download missing media.
  The app does not yet expose every messaging operation in the Quickshell reference
  client. Live phone and provider validation remains separate from its private
  display tests.
- The Quickshell example is a reference client, not a stable desktop application.
- Unix-socket IPC uses protocol 1 and is experimental for outside clients.
- IPC snapshots are bounded to 16 MiB and 256 chunks. Ordinary batches target
  1 MiB; individual normalized records can use a 2 MiB line. Larger records or
  assemblies fail instead of being truncated. See the [IPC contract](ipc.md).

## Google Messages

- The native client requires Google Messages on the phone and Google's service.
  It is not a direct carrier RCS implementation.
- Capabilities depend on the connected phone and selected helper. The native
  helper does not support creating conversations, deleting messages, sending
  typing notifications. Mark-read refreshes phone state before reporting the
  resulting unread flag. Incoming typing supports direct
  RCS conversations with one configured RCS SIM and an exact known sender match.
  Group and encrypted composing events are not supported.
  See the [incoming typing investigation](gmessages-typing.md).
- Message state and fetched history pages can persist across daemon restarts.
  The live cache and on-disk archive have limits; stored history is not a
  complete copy of the provider's history.

## Packaging

Release tarballs, source installation, and [Arch PKGBUILDs](../packaging/arch/README.md)
are available. The Arch packages are not published to the AUR. Optional browser
setup requires Python 3 and an installed Chromium browser. Google sign-in
compatibility depends on the browser and Google's authentication policy.
