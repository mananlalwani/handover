# Desktop IPC protocol 1

`handoverd` owns runtime state. Desktop clients read it through the Unix socket
at `$XDG_RUNTIME_DIR/handover/handoverd.sock`. The daemon creates the directory
with mode `0700` and the socket with mode `0600`. This is a local user interface,
not a network API. `XDG_RUNTIME_DIR` must be absolute.

The protocol remains experimental before Handover 1.0. The authoritative types
are [`Request`, `Method`, and `ServerPayload`](../crates/handover-ipc/src/lib.rs).
Their device, notification, media, call, and messaging fields use
[`handover-core`](../crates/handover-core/src/lib.rs) models. IDs are opaque.
Keep every component of a scoped ID and return it unchanged in commands.

## Framing and limits

Each record is one UTF-8 JSON object followed by a newline. Every request and
response carries `"protocol":1`. A socket read can contain part of a record or
several records. Retain partial bytes until their newline, including when other
async work completes. Escaped newlines inside strings are JSON content.

Collection responses with chunk variants and snapshot chunks target 1 MiB. An individual
normalized record can use the 2 MiB hard line limit. This headroom matters when
a helper event fits its own 1 MiB limit but grows as normalization adds scoped
IDs or sender details. Writers reserve a byte for the trailing newline. Readers
must accept the larger records while bounding incomplete input. Older clients
that only accept 1 MiB cannot read these exceptional records.

Subscription and recovery snapshots have aggregate limits of 16 MiB of JSON
content and 256 chunks. Count envelopes as well as records. Reject an excessive
or interrupted assembly and reconnect. Never publish part of a snapshot or
silently discard a record to make a response fit. Rust clients return
`IpcError::SnapshotTooLarge` on aggregate overflow and `IpcError::LineTooLong` on
line overflow. Quickshell guards completed lines and retained snapshot state;
its `SplitParser` owns buffering before the newline arrives.

The daemon admits at most 64 IPC connections. Pre-subscription reads and writes
have 15-second deadlines. A slow connection can be closed without stopping other
clients. Use bounded work queues and let the daemon send a replacement snapshot
when event consumption falls behind.

## Starting a client

A version check uses:

```json
{"protocol":1,"method":"hello"}
```

The response is:

```json
{"protocol":1,"type":"hello","supported_protocols":[1]}
```

Treat an unsupported version as unusable. Do not reinterpret unknown state or
capability values as familiar ones.

Use one connection for subscription state and another for commands. Keep one
command in flight on a command connection. Requests have no general request ID;
subscription events can otherwise arrive between a command and its reply.
The Rust `Client` convenience methods expect their corresponding response.

A subscription request is:

```json
{"protocol":1,"method":"subscribe","shares":true,"media":true,"messages":false}
```

All three flags are required. Devices, notifications, and calls are included.
`media` adds media sessions. `messages` adds messaging accounts, conversations,
typing/read state, and messaging events. `shares` enables transient received,
progress, and result events. `audio.subscribe` separately enables audio state.
Contacts and message history have their own requests.

The initial response is `subscribed`, or a contiguous sequence of
`subscribed_chunk` records. Its collections are:

- `devices`, `notifications`, `media_sessions`, and `calls`;
- `messaging_accounts`, `conversations`, `typing_states`, and `read_states`.

Each chunk has `done:false` until the final `done:true`. Concatenate each
collection in order, then replace the published view once. Do not combine
`subscribed_chunk` with `snapshot_chunk`, or accept another response or event
inside an unfinished sequence.

## Events and recovery

After subscription, apply `device_added`, `device_updated`, and `device_removed`
by device ID. Notifications and media use the same added/updated/removed pattern
with their full scoped IDs. Calls use `call_updated` and `call_removed`.
Messaging events carry account, conversation, or message scope. Share results,
clipboard events, and command outcomes are transient, not a persistent archive.

Subscription registers its event receiver before taking the initial snapshot.
An event can therefore repeat state already present in that snapshot. Upsert by
ID; do not duplicate entries or replay historical alerts just because a client
reconnected.

The daemon's broadcast channel retains 64 events. A lagging subscriber receives
`snapshot`, or contiguous `snapshot_chunk` records, containing current state for
its subscribed collections. Replace that state after the whole snapshot arrives.
The daemon does not promise to replay each missed transition or transient event.
Messaging outgoing-operation state and audio state can follow separately.

On EOF, malformed framing, unsupported protocol, or incomplete assembly, discard
the connection and its partial buffers. Reconnect and subscribe again. Cached
views must not become authoritative, and previously accepted commands must not
be resent as part of recovery.

The same Rust assembly budget also applies to complete-list responses. These include `notifications_chunk`, `media_chunk`, `contacts_chunk`, `conversations_chunk`,
and `outgoing_operations`, each with `done` marking completion. These are distinct
from subscription snapshot chunks. Message history uses `messages.history` and
returns `history` with an optional `cursor_next`. Treat that cursor as opaque;
stop paging when it is absent or repeats. History pages can contain one large
message, so the hard line limit applies even though ordinary pages target a
smaller budget.

## Acceptance and outcomes

The response name alone does not prove the phone performed an operation:

| Response | Meaning |
| --- | --- |
| `command_completed` | A notification command was accepted for delivery to the phone. |
| `media_accepted` | A media command was accepted; wait for a media update to observe playback. |
| `share_accepted` | The backend accepted a share and may provide a transfer ID. |
| `share_progress` | Bytes were written to the authenticated stream, not confirmed stored by the receiver. |
| `share_result` | The backend reported the transfer's terminal outcome. |
| `message_accepted` | A messaging command has an operation ID; later state carries its outcome. |

A timeout or disconnect during a command can leave its outcome unknown. Do not
retry a send, action, or other mutation automatically. Return the uncertainty to
the caller. Use later daemon state or operation evidence to resolve it.

Errors carry `type:"error"`, a stable `code`, and a human-readable `message`.
Some messaging failures also carry `conversation_id`; only the matching history
load should fail. Logs should contain fixed diagnostics and operation identifiers,
not message bodies, notification text, paths, credentials, or rejected payloads.
