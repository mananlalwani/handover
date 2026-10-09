# Design

Handover provides shared Android device state and actions to Linux desktop
clients. The daemon owns runtime state; clients rebuild their views from it.

Frontends consume normalized Handover types. Backend names, object paths,
plugin IDs, D-Bus interfaces, and Google protocol objects stay inside their
adapters.

The native Android backend feeds normalized events into `StateStore`. Clients
consume the same backend-independent IPC models.

## Native Android backend

The native backend publishes normalized device events into `StateStore`.
Native device IDs are namespaced as `native:<certificate fingerprint>`.
Clients consume `Device`, `BatteryState`, capabilities, and the existing
snapshot/event IPC.

The Linux native adapter persists its self-signed identity certificate and key,
plus a peer allowlist, below `${XDG_STATE_HOME:-~/.local/state}/handover/native`
with restrictive permissions. The Android companion persists its installation
identity in app-private storage and keeps the private key in Android Keystore.
A peer is trusted only after both sides compare the eight-digit code and
approve it. Each pairing attempt generates a new code. Each hello commits SHA-256 of a
random 16-byte nonce. Both sides reveal the nonce in `pair_open`. The code
comes from the two certificate fingerprints with each nonce bound to its
owner. Discovery never creates a device or a trust record.

The native listener advertises `_handover._tcp.local.` through DNS-SD and binds
TCP port `24837`. The advertisement record has unit tests. Most live discovery
testing has used manual `address:port` entry, including Tailscale.
TLS 1.3 with peer certificates authenticates the
stream. The stored fingerprint is the pairing decision. Discovery addresses
and TXT values are untrusted hints.

The application protocol uses a four-byte big-endian length followed by a
versioned JSON message. Frames cap at 64 KiB. Native messages include:
`hello`, `pair_open`, `pair_confirm`, `paired`, `battery`, `connectivity`,
`notification_post`, `notification_removed`, `notifications_sync`,
`notifications_request`, `notification_dismiss`, `notification_reply`,
`notification_action`, `media_post`, `media_removed`, `media_sync`,
`media_request`, `media_control`, `revoke`, `ring`, `user_ping`, `ping`, and
`pong`.

An Android `hello` may include the previously trusted server ID. If Linux has
revoked that phone, it replies with `revoke` so Android clears its stale pin
before presenting a new pairing request.

The daemon rejects unknown versions, oversized frames, malformed identities,
unpaired fingerprints, commitment-less hellos from unknown peers, openings
that do not match the committed nonce, and confirmations that do not repeat
the displayed code. A failed pairing attempt drops the session so a retry
requires a fresh comparison. Session count, read/write timeouts, and frame
size are bounded. Before publishing updates, the daemon validates
`BatteryState` and checks bounds on notification fields and actions, media
players, metadata, and controls.
Notification content and track metadata are never sent before the peer is
paired, and titles/bodies/artists are excluded from normal logs on both
endpoints.

Daemon IPC provides `native.peers`, `native.pending`, `native.pair`, and
`native.unpair`. Unpairing removes the local allowlist entry, closes the
active stream, and blocks that peer from reconnecting. Revocation is local to
each endpoint. Remove the pairing on both sides to clear both trust records.

The Android app uses an explicit user action to start its
`connectedDevice` foreground service. The service owns the active transport,
DNS-SD registration/discovery, reconnect attempts, and event-driven battery
observation. Android lifecycle restarts and network changes recreate the
connection from the persisted identity and allowlist. They do not bypass
pairing.

## Clipboard boundary

Native Handover owns explicit clipboard transfer on the primary path. Bounded
text, HTML, URI, and small file-backed items move over the paired TLS session.
Contents never enter logs or ordinary daemon snapshots.

Optional Android background mirroring is an in-app setting while the foreground
service runs. Linux-to-phone send uses `handoverctl clipboard` or the IPC
equivalent.

Handover persists up to 25 recent native phone-to-Linux text
entries and 25 user-pinned strings in a mode-0600 state file. The serialized
history is limited to 900 KiB so it fits within one local IPC response.
Dedicated clipboard-history IPC methods expose it to local clients.

Android 10 and later restrict background clipboard reads. Handover provides a
notification action and a Quick Settings tile for explicit sends. Its optional
automatic path requires a user-granted `READ_LOGS` permission and display-over-
apps access. It filters `ClipboardService` output for its package name, opens a
transient activity for one permitted read, and does not retain log lines.
Clipboard clips marked sensitive by Android are excluded from automatic sends.

## Runtime state

`handoverd` owns the authoritative in-memory device and active notification
maps. It applies normalized events synchronously, logs meaningful state
transitions, and publishes the result to clients. The runtime device,
notification, and media-session maps are in-memory only. The native backend's
own identity certificate/key and peer allowlist are persisted as described in
the native section above.

## File and URL handoff

The daemon routes file and URL shares to one explicitly selected, paired native
device. URLs travel in a bounded control frame. Files are copied into private
staging before queueing, then streamed over the authenticated TLS session in
bounded chunks. Acceptance means the daemon queued the request. The receiver
reports completion or a normalized failure when it handles the transfer.

Received files use a safe basename and a private temporary file under
`${XDG_STATE_HOME:-~/.local/state}/handover/native/received`. The daemon
publishes the share only after all bytes arrive and the size matches. Errors,
timeouts, and disconnects remove partial files. Handover never opens or runs a
received file automatically. Incoming and outgoing share events are transient;
they do not appear in snapshots or a transfer history.

## Notifications

Notification IDs combine the native device ID and the phone's notification key.
The normalized record carries app name, title, body, clearable state, actions,
and reply availability. The Android listener publishes updates and removals,
and sends a full snapshot after pairing, listener reconnect, or a daemon
request. Revoking notification access clears stale entries.

The daemon checks current notification state before queueing a dismissal,
action, or reply. Acceptance means the command entered the live session queue;
it does not confirm that Android or the source app acted. Subsequent notification
events remain the source of truth.

## Media sessions

Android's `MediaSessionManager` publishes active sessions and supported
actions over the authenticated native connection. A session ID combines the
source device with its package name. The model carries playback state,
metadata, position, duration, and supported controls. It does not carry volume
or album art. Relative seeks are not advertised because Android has no matching
platform action.

The daemon validates each media command against the current session, device
connection, and advertised controls. It queues accepted commands for the live
session and waits for later media updates to reflect playback state. Disconnects
and revoked notification-listener access remove stale sessions.

## Messaging

Messaging conversations belong to messaging accounts (endpoints such as one
linked Google Messages session), never to physical devices. The public
model lives in `handover-core::messaging`: account-scoped conversation and
message ids with opaque backend-local keys, RCS-first transports, attested
capability sets (absence means "not attested", never "assumed absent"),
and accepted/sent/delivered/displayed/failed semantics where acceptance is
never delivery. There are no edit, membership-change, or
disappearing-message capabilities because no backend attests them.

The Google Messages path runs through an optional separate helper process
over a versioned JSON contract (see
[`docs/gmessages-helper.md`](docs/gmessages-helper.md)). The helper owns
credential bundles, pairing, relay RPCs, receive streams, and media transfer; the
daemon owns normalized accounts, conversations, bounded message windows
(300 per conversation), statuses, typing, and read state, and validates
every helper record and outbound command. Google protocol types stay below
the normalized model; no AGPL material enters the MIT tree. A missing or dead helper only marks its
accounts offline; device state is untouched.

The daemon caches normalized accounts, conversations, message windows, and read
state in `handover/messaging-cache.json` under its state directory. Restored
accounts start disconnected and unauthenticated until the helper reports their
current state. `HANDOVER_MESSAGING_CACHE=0` disables cache loading and writes.

Clients use additive protocol-1 `messages.*` methods (accounts,
conversations, history with cursor paging, send/send-file, react/unreact,
read, typing-start, delete, open, login/logout, sync) with an opt-in
`messages` subscribe flag, following the same snapshot-recovery and
bounded-channel rules as devices, notifications, and media. Login bundles
travel file/stdin → local socket → local helper pipe only. Native authentication
uses desktop Secret Service; pairing records use private helper-owned files.
Confirmed native logout waits for phone unpairing before deleting credentials.

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

The developer install layout is `~/.local` via `make install-user`. The Arch
package puts binaries in `/usr/bin` and the user unit in
`/usr/lib/systemd/user/handoverd.service`.

## Local IPC

`handoverd` listens on
`$XDG_RUNTIME_DIR/handover/handoverd.sock`. It creates the directory with mode
`0700` and the socket with mode `0600`. The daemon removes a stale socket only
when the existing path is a socket and no process accepts connections there.

Protocol 1 uses newline-delimited JSON with a 1 MiB line limit. Every request
includes `protocol: 1` and a method. Basic methods include:

- `hello`
- `devices.list`
- `subscribe`

Other methods cover notifications, media, sharing, pairing, messaging, and
device actions. The complete request types are defined in
[`handover-ipc`](crates/handover-ipc/src/lib.rs).

Server messages also carry `protocol: 1`. Snapshots serialize normalized
devices and notifications, plus media and messaging state when requested. Battery
deserialization still runs the core model's percentage validation. Device and
notification events use add/update/remove forms. Transport identities and
authentication material stay below the public socket contract.

A subscription response includes current devices and notifications, closing
the race between fetching state and beginning event delivery. The daemon then
sends future normalized events. Each client runs in its own task, so a broken
client cannot stop the backend or other clients.

The event channel retains 64 messages. A slow subscriber that falls behind
does not block device updates. Once it reads again, the server replaces missed
history with a current snapshot for the subscribed collections. Large snapshots
are split into chunks to respect the line limit. Current state
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
