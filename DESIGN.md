# Design

Handover is Linux desktop infrastructure. Its main job is to make device state
and actions available consistently across the desktop, not to act primarily as
a phone-management application.

Frontends consume normalized Handover device state. KDE Connect object paths,
plugin names, D-Bus interfaces, and event formats belong inside the KDE Connect
backend and must not become frontend concepts.

Backends are replaceable. The core domain model describes what Handover knows
about a device without requiring a particular discovery or transport system.
The initial KDE Connect backend proves this boundary but does not define it.

The current KDE Connect path is:

```text
kdeconnectd
    -> session D-Bus
    -> handover-kdeconnect
    -> handover-core StateEvent
    -> handoverd device and notification maps
    -> Unix socket
    -> handoverctl and Quickshell
```

`handover-kdeconnect` owns KDE Connect service names, object paths, interfaces,
properties, and signal decoding. It converts each D-Bus snapshot into a
validated `handover-core::Device`. It watches KDE Connect's D-Bus owner and
re-enumerates after a restart.

## Native Android backend

The native backend is a second provider of the same normalized device events.
`handoverd` starts the native listener alongside the KDE Connect adapter and
merges both event streams into its existing `StateStore`. Native device IDs are
namespaced as `native:<certificate fingerprint>`, while KDE Connect IDs remain
unchanged. Clients continue to consume `Device`, `BatteryState`, capabilities,
and the existing snapshot/event IPC; no client selects or identifies a backend.

The Linux native adapter persists its self-signed identity certificate and key,
plus a peer allowlist, below `${XDG_STATE_HOME:-~/.local/state}/handover/native`
with restrictive permissions. The Android companion persists its installation
identity in app-private storage and keeps the private key in Android Keystore
when the transport implementation is enabled. A peer is only trusted after the
two users compare the displayed eight-digit code and explicitly approve it on
their respective sides. The code is fresh for every ceremony: each hello
carries a SHA-256 commitment to a random 16-byte nonce, both sides reveal
their nonces in `pair_open`, and the code is derived from the two certificate
fingerprints with each nonce bound to its fingerprint owner. A discovery
result alone never creates a device or trust record.

The native listener advertises `_handover._tcp.local.` through DNS-SD and binds
TCP port `24837`. The advertisement record has unit-test coverage, but no live
multicast discovery verification has been performed; manual `address:port`
entry is the verified path. The Android app also accepts an explicit `address:port` when
multicast discovery is unavailable, including over a user-managed Tailscale
connection. TLS 1.3 with peer certificates authenticates and
encrypts the stream; the stored certificate fingerprint supplies the pairing
decision. Discovery addresses and TXT values are treated as untrusted hints.
The application protocol uses a four-byte big-endian length followed by a
versioned JSON message, with a 64 KiB maximum frame. The native messages are
`hello`, `pair_open`, `pair_confirm`, `paired`, `battery`, `notification_post`,
`notification_removed`, `notifications_sync`, `notifications_request`,
`notification_dismiss`, `notification_reply`, `notification_action`,
`media_post`, `media_removed`, `media_sync`, `media_request`, `media_control`,
`revoke`, `ping`, and `pong`.
An Android `hello` may include the previously trusted server ID. If Linux has
revoked that phone, it replies with `revoke` so Android clears its stale pin
before presenting a new pairing request.
Unknown versions, oversized frames, malformed identities, unpaired
fingerprints, commitment-less hellos from unknown peers, openings that do not
match the committed nonce, and confirmations that do not repeat the displayed
code are rejected; a failed ceremony drops the session so a retry starts a
fresh, user-visible ceremony. Session count, read/write timeouts, and frame
size are bounded. The daemon accepts battery, notification, and media updates
only after `BatteryState` validation (battery), field/action bound checks
(notifications), or player/metadata/control bound checks (media), then
publishes the ordinary device/notification/media updates to all clients.
Notification content and track metadata are never sent before the peer is
paired, and titles/bodies/artists are excluded from normal logs on both
endpoints.

Native peer administration is exposed through the existing daemon IPC as
`native.peers`, `native.pending`, `native.pair`, and `native.unpair`. Unpairing
removes the local allowlist entry, closes the active stream, and prevents a
stale peer from reconnecting. Revocation is local to each endpoint and must be
performed on both sides when both trust stores need to be cleared.

The Android app uses an explicit user action to start its
`connectedDevice` foreground service. The service owns the active transport,
DNS-SD registration/discovery, reconnect attempts, and event-driven battery
observation. Android lifecycle restarts and network changes recreate the
connection from the persisted identity and allowlist; they do not bypass
pairing. The API constraints and source links are recorded in
[`docs/research/native-backend-apis.md`](docs/research/native-backend-apis.md).

KDE Connect remains optional and independent. Its disappearance removes only
KDE-derived runtime entries; a native paired phone remains present and can
continue reporting state. Re-enabling KDE Connect may restore its own device
entry alongside the native entry.

## Clipboard boundary

The current clipboard path is owned by KDE Connect itself:

```text
Android Clipboard
    <-> KDE Connect clipboard plugin and transport
    <-> Linux system clipboard
```

The desktop plugin reads and writes the Linux system clipboard and exchanges
text packets with the Android plugin. The per-device D-Bus interface exposes a
`sendClipboard()` request for Linux-to-device transfer, but it does not expose
remote clipboard text or a signal for remote text updates. Those updates are
handled internally by `kdeconnectd`.

Handover therefore does not duplicate this transport or add a separate
clipboard engine at present. It does not persist clipboard contents, log them,
or include them in normal IPC snapshots. KDE Connect's clipboard plugin must
be enabled for the device.

KDE Connect suppresses clipboard write-back by comparing content and type,
without a timer. Its enabled per-device plugins receive local changes, so a
Linux copy can reach multiple connected devices. Remote writes share one Linux
clipboard; differing simultaneous updates are last-writer-wins. Handover does
not select a default device or maintain a clipboard history.

Android 10 and later restrict background clipboard reads. KDE Connect can use
its foreground "Send clipboard" action for the Android-to-Linux direction; its
automatic path requires the user-granted privileged `READ_LOGS` setup. Linux-
to-Android transfer can remain automatic when the plugin is enabled. This is an
Android platform restriction, not a Handover protocol workaround.

For future work that needs daemon-owned Wayland clipboard access, the platform
boundary belongs outside `handover-core`. The current Wayland options are the
event-driven `ext-data-control-v1` protocol, with `wlr-data-control` as a
compatibility path. No such Handover provider is implemented yet.

`handoverd` owns the authoritative in-memory device and active notification
maps. It applies normalized events synchronously, logs meaningful state
transitions, and publishes the result to clients. The runtime device,
notification, and media-session maps are in-memory only. The native backend's
own identity certificate/key and peer allowlist are persisted as described in
the native section above; KDE Connect remains responsible for its pairing
data.

## File and URL handoff

The KDE Connect share plugin accepts outgoing URLs and local-file URLs through
its per-device D-Bus object. Handover validates an explicit paired, connected,
share-capable target, then forwards one resource through that adapter. A
successful D-Bus return means the request was accepted, not delivered.

KDE Connect owns incoming file writes, destination naming, and incoming URL
handling. Its `shareReceived` signal gives Handover a coarse event with the
source device after an incoming file has been saved or a URL has been handed
to the desktop handler. The signal exposes no active transfer ID, progress,
failure, or cancellation. Handover therefore has `ReceivedShare` and
`SharedResource` types but no speculative `Transfer` state machine. These
events are transient: the daemon broadcasts them without adding a transfer
history to snapshots. Slow or disconnected clients may miss the display event,
but KDE Connect's file write or URL handling does not depend on a UI client.

KDE Connect also emits this signal for a temporary file when the user chooses
to open shared text in an editor. A file event alone does not prove that
Android sent a file payload.

KDE Connect may open an incoming file when the sender sets its `open` flag;
Handover does not open files and cannot override that upstream behavior through
the current D-Bus interface.

The socket's `share.url` and `share.file` requests use protocol 1. `share.file`
contains a standard local `file://` URL, which preserves spaces and Unicode
without shell parsing; the daemon checks that it identifies a readable regular
file. `share_accepted` confirms only D-Bus acceptance. Incoming events use
`share_received`. Subscribers request these new transient events with
`"shares": true`; older protocol-1 subscribers omit the flag and continue to
receive only device and notification events. Normal logs and
`handoverctl monitor` report resource kind and source device, not file
contents, full paths, or URL queries.

Native sharing uses these same daemon, CLI, and Quickshell APIs. The daemon
selects the explicitly requested paired device and routes the request to its
available backend; a native and a KDE Connect representation may therefore
coexist without exposing backend names to clients. Native share messages are
sent only on an already authenticated, paired TLS 1.3 session. A URL is one
bounded control frame, `{"type":"share_url","protocol":1,"url":"..."}`.
For a file, the sender first sends
`{"type":"share_file","protocol":1,"name":"...","size":N}` and then
exactly `N` raw bytes on the same TLS stream. The JSON frame remains subject to
the 64 KiB control-frame limit; file bytes never enter a JSON frame.

Native file transfers are limited to 100 MiB and use at most a 32 KiB transfer
buffer. The advertised name must be one safe basename, at most 255 UTF-8
bytes: path separators, `.` and `..`, NUL, control characters, and invalid
UTF-8 are rejected. The receiver writes into a private temporary file below
`${XDG_STATE_HOME:-~/.local/state}/handover/received`, then atomically renames
it to the sanitized final name only after all bytes arrive and the size matches.
Temporary files are removed on cancellation, EOF, timeout, size mismatch,
authentication failure, or any other transfer error; an interrupted transfer
never leaves a usable partial file. Received files are never opened or executed
automatically.

The native sender's `share_accepted` result means only that the authenticated
peer queued the request. Completion is represented by a transient
`share_received` event after a URL is accepted or a file is fully committed;
there is no completion acknowledgement in the wire protocol, and failures are
reported as failed command/transfer results where the existing API supports
them. The event includes the normalized source-device identity, so clients do
not infer it from a path or transport. Native and KDE Connect incoming shares
both use the existing transient event subscription and do not create transfer
history.

Notifications use an ID made from the source `DeviceId` and a device-local
notification ID. This prevents collisions between phones. The normalized
record carries app name, title, body, optional icon path, clearable state,
optional actions, and reply availability. KDE Connect's D-Bus API does not
currently expose action identifiers or labels, so its adapter publishes an
empty action list. It does expose reply and dismissal operations.

Clients request dismissal, action invocation, or reply over IPC. The daemon
checks the current notification and its advertised capabilities before the
KDE adapter makes a D-Bus call. A successful response means KDE Connect
accepted the call, not that Android or the app confirmed delivery. Commands
do not optimistically remove daemon state; subsequent KDE Connect signals
update it.

The native path carries the same normalized records over the paired TLS
session. The Android app reads the platform notification stream through its
notification-listener service (permission-gated; a denied permission reports
`enabled: false` so the daemon clears stale entries instead of showing
ghosts) and sends `notification_post` upserts, `notification_removed`
retractions, and full `notifications_sync` snapshots after pairing, on
listener reconnect, and on daemon request. The phone's notification key
becomes the Handover `local_id` inside the existing `native:<fingerprint>`
device scope, so native and KDE Connect entries coexist without collisions
and clients never select a backend. Android actions map by index to the
existing id/label pairs with no invented capabilities; the first
free-form-input action advertises `reply_supported`, and `clearable` follows
the platform flag. Desktop dismissal, action, and reply commands are queued
for the live session and report IPC acceptance exactly like the KDE path:
success means the command reached the session, not that Android confirmed
the effect. A disconnect removes the peer's native notifications so a
reconnect resyncs from current phone state.

## Media sessions

Media state follows the same backend boundary:

```text
Android media sessions
    -> KDE Connect mprisremote D-Bus object
    -> existing KDE Connect MPRIS player objects
    -> handover-kdeconnect
    -> normalized MediaSession events
    -> handoverd media-session map
    -> Unix socket clients
```

`handover-core` exposes `MediaSessionId`, `MediaSession`, `PlaybackState`,
`MediaControl`, `MediaEvent`, and `MediaCommand`. A session ID is the pair of
the source `DeviceId` and a device-local opaque player identifier. The KDE
adapter currently uses KDE Connect's player name as that device-local value,
so the same name on two phones cannot collide. Position and duration are milliseconds;
available volume is normalized to an integer percentage. Album art is not
copied into the Handover model or ordinary IPC messages.

The KDE adapter treats the `mprisremote` `playerList` as the authoritative set
of current players. It reads per-player metadata, playback state, position,
duration, volume, and control capabilities from KDE Connect's dynamically
created standard MPRIS services. KDE service names, object paths, and action
details remain inside `handover-kdeconnect`; clients treat `player_id` as opaque.
KDE's synthesized MPRIS identity combines player and device display names.
If two devices share a name, the adapter withholds their sessions rather than
risk routing a command to the wrong phone. KDE's MPRIS position getter uses
the selected player, so Handover reports position only for that player.
KDE does not expose trustworthy stop or volume-control capability flags, so
Handover keeps volume read-only and does not expose those commands yet.

`handoverd` owns the authoritative in-memory media-session map. It removes
sessions when KDE Connect no longer advertises them or when their device
disconnects, and rebuilds them after KDE Connect restarts. Media state is not
persisted. Snapshots include all current media sessions, and subscriptions
receive `media_added`, `media_updated`, and `media_removed` events through the
existing bounded channel and snapshot-recovery behavior.

Media controls are validated against the current session, connected paired
device, advertised media capability, and the session's normalized controls.
The D-Bus call is a request: `media_accepted` means KDE Connect accepted it,
not that Android has already changed playback. The subsequent media update is
the source of truth. Unsupported controls, unknown sessions, disconnected
devices, and invalid numeric values return controlled protocol errors.

The native path carries the same normalized sessions over the paired TLS
session:

```text
Android MediaSessionManager active sessions
    -> native media observer (notification-listener component)
    -> media_post / media_removed / media_sync frames
    -> normalized MediaSession events
    -> handoverd media-session map
    -> Unix socket clients
```

The phone's package name becomes the Handover `player_id` inside the
existing `native:<fingerprint>` device scope, so native and KDE Connect
sessions coexist without collisions and clients never select a backend.
Playback maps playing/paused/stopped directly; transitional states report
unknown rather than guessing. Only the controls behind the platform actions
bitmask are advertised (absolute `seekTo` maps to `SetPosition`; relative
seeks have no genuine platform API and are rejected if advertised), with the
combined toggle derived exactly like the KDE adapter. Position and duration
are reported only when the platform supplies them; volume is never
transported. Desktop commands are queued for the live session and report IPC
acceptance exactly like the KDE path; a disconnect removes the peer's native
sessions so a reconnect resyncs, and a revoked listener clears them with the
notifications.

KDE Connect already exports remote Android players through standard MPRIS
services named `org.mpris.MediaPlayer2.kdeconnect.mpris_<id>`. Handover should
not expose another MPRIS player for the same sessions because that would
duplicate players in desktop media menus. The current public integration is
the Handover IPC API and its CLI/Quickshell clients; a future MPRIS bridge, if
needed for a backend-independent use case, should be a separate deliberate
integration rather than daemon-owned duplicate export.

## User service

The installed daemon runs as a systemd user service enabled under
`default.target`. It uses the user's session D-Bus and `$XDG_RUNTIME_DIR`;
it never runs as root. It does not depend on a compositor or on a desktop
activating `graphical-session.target`. `Restart=on-failure` recovers from
unexpected exits after a short delay without restarting after a clean stop.

systemd creates `$XDG_RUNTIME_DIR/handover` with mode `0700`, while the daemon
continues to own and clean up `handoverd.sock`. The unit also sets a restrictive
umask and prevents the process from gaining new privileges. More invasive
sandboxing is intentionally deferred because Handover is desktop
infrastructure and needs session-bus and runtime-directory access.

The developer install layout is:

- `~/.local/bin/handoverd`
- `~/.local/bin/handoverctl`
- `${XDG_DATA_HOME:-~/.local/share}/systemd/user/handoverd.service`
- `${XDG_DATA_HOME:-~/.local/share}/handover/quickshell/` for the optional
  reference client

## Local IPC

`handoverd` listens on
`$XDG_RUNTIME_DIR/handover/handoverd.sock`. It creates the directory with mode
`0700` and the socket with mode `0600`. The daemon removes a stale socket only
when the existing path is a socket and no process accepts connections there.

Protocol 1 uses newline-delimited JSON. Every request includes `protocol: 1`
and one of these methods:

- `hello`
- `devices.list`
- `subscribe`

Server messages also carry `protocol: 1`. Snapshots serialize normalized
`handover-core::Device` and `Notification` values directly. Battery
deserialization still runs the core model's percentage validation. Device and
notification events use add/update/remove forms; no KDE Connect object paths
or reply tokens cross the socket.

A subscription response includes current devices and notifications, closing
the race between fetching state and beginning event delivery. The daemon then
sends future normalized events. Each client runs in its own task, so a broken
client cannot stop the backend or other clients.

The event channel retains 64 messages. A slow subscriber that falls behind
does not block device updates. Once it reads again, the server replaces missed
history with a current `snapshot` message containing both maps. Current state
matters more than replaying every intermediate battery reading or notification
update.

The daemon shares its maps through one `RwLock`. Backend callbacks take a short
write lock, and IPC snapshots take a short read lock and clone the state.
No lock guard crosses an async wait.

`handoverctl devices` makes a one-shot request. `handoverctl monitor` reconnects
every two seconds after daemon loss. The Quickshell singleton follows the same
fixed retry interval, requests a new snapshot after each connection, and owns
only disposable view state.

Future UI clients must be safe to close, restart, or replace without affecting
daemon state.

Quickshell is the first reference frontend. It demonstrates the public IPC and
domain model, but neither the daemon nor the libraries depend on Quickshell.
