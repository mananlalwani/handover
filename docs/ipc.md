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

## Compatibility policy

Protocol 1 is experimental. Install matching daemon and client releases while
this contract is evolving; the version number alone does not guarantee that an
older client understands every current response.

- Unknown object fields may be ignored. Add a field within protocol 1 only when
  its absence preserves existing behavior and old readers can ignore it safely.
  New readers must supply an explicit default or accept absence for that field.
- Required fields, record shapes, units, ID scope, ordering, and the meaning of
  an accepted command are part of the contract. Changes to these need a protocol
  migration rather than a new interpretation of existing data.
- A new method can be optional when clients request it explicitly and handle
  rejection by an older daemon. New response types or enum values are not generally
  compatible with old clients. Rust readers reject unknown variants; Quickshell
  warns and ignores unknown top-level message types outside an assembly. Do not
  depend on that difference to negotiate a capability or infer a known state.
- Every envelope must use a supported `protocol`. `hello` reports
  `supported_protocols`; the Rust `Client::hello` returns that list for its caller
  to check, as the CLI does. It does not automatically select or downgrade a
  version. Never retry a mutation with a different protocol after an uncertain
  response.
- Check compatibility changes across the daemon, Rust client, CLI, desktop
  bridge, and Quickshell. An Android transport change also needs its own wire
  validation; desktop IPC versioning does not version the phone protocol.

Protocol 1 remains experimental. Compatibility checks run with the workspace
and Quickshell test suites.

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

The reference Quickshell client uses an unsubscribed connection for each ordinary
request, with a limit of 16 concurrent requests and no queued mutations. Request
errors and partial lists stay on their owning connection. It does not retry a
mutation when its response is lost. This is a client implementation choice;
protocol 1 does not add request IDs or require a new socket for every command.

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
Before taking the recovery snapshot, the daemon drops the stale event backlog
and registers for new events. Events arriving during snapshot delivery follow it;
superseded events do not replay older values over the replacement state.
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

## Local messaging workspace

These optional protocol 1 methods use normalized account, conversation, and
message IDs. Install matching clients and daemon; they add response variants to
the experimental contract. They do not require a connected helper. Unknown
accounts cannot access search results, context, or drafts.

| Method | Request fields | Response type |
| --- | --- | --- |
| `messages.search_conversation` | `conversation_id`, `query`, `offset` default 0 | `message_search` with `hits`, `has_more`, scoped to that conversation |
| `messages.attachment_draft` | `conversation_id` | `message_attachment_draft` with `draft` |
| `messages.save_attachment_draft` | `conversation_id`, optional `file_url`, optional `mime`, `revision` | `message_attachment_draft` with saved `draft` |
| `messages.search` | `query`, optional `account_id`, `offset` default 0 | `message_search` with `hits`, `has_more` |
| `messages.media` | `conversation_id`, `offset` default 0, maximum 10000 | `message_media` with `conversation_id`, `messages`, `has_more`, `done` |
| `messages.previews` | `conversation_ids`, at most 50 | `message_previews` with `messages`, `done` |
| `messages.context` | `message_id` | `message_context` with `conversation_id`, `messages`, `done`, optional `cursor_next` |
| `messages.conversation_settings` | none | `conversation_settings` with `settings` list |
| `messages.save_conversation_settings` | `settings` | `conversation_settings_updated` with saved `settings` |
| `messages.workspace` | none | `messaging_workspace` with `workspace` |
| `messages.select` | `location`, or null to clear | `messaging_workspace` with `workspace` |
| `messages.draft` | `conversation_id` | `message_draft` with `draft` |
| `messages.save_draft` | `draft` | `message_draft` with saved `draft` |
| `messages.submit_draft` | `draft` | `message_accepted` with durable operation `request_id` |
| `messages.resolve_draft_send` | bound `draft` | `message_draft` after clearing a confirmed sent draft or restoring it for review |

Conversation settings contain `conversation_id`, `revision`, `pinned`, `archived`,
`muted`, and `unread_reminder`. All flags default to false at revision zero.
They organize Handover's desktop views and alerts only. They do not represent
provider capabilities, change phone read state, or belong to helper IPC.

Saving requires the current revision and a known account. The workspace assigns
a new revision, persists the record, and broadcasts `conversation_settings_updated`
to messaging subscribers. Stale revisions are rejected. Clients merge updates by
revision and read the settings list after subscribing and after recovery snapshots;
provider snapshots do not contain these workspace records. Account removal clears
its settings. The list has at most 512 records, including cleared records retained
to reject stale writes. Account IDs are limited to 128 bytes and thread IDs to 256
bytes. The normal IPC line limit applies. Storage follows the workspace's private
SQLite or session-only mode.

Search reads saved history. Queries have at most 256 characters and 16 literal
prefix terms. Each page contains at most 25 hits; offset is limited to 10,000.
Context returns up to 22 saved messages around the anchor. Context responses
can span several frames; collect them until `done: true` under the normal
aggregate and line limits. Read `cursor_next` from the terminal context frame
and pass it unchanged to `messages.history` to continue toward older messages.
The daemon returns a continuation only when saved page metadata establishes one.
An omitted cursor does not establish that the provider has no older history.

Media listing reads only the saved archive for the requested account and
conversation. Each page has at most 16 messages with attachments, newest first
by timestamp and message ID. Deleted and text-only messages are omitted.
Increment offset by 16 for the next page. Collect chunked frames until `done`
and use `has_more` from that terminal frame. The archive may change between
requests. This operation does not contact a provider or download attachment bytes.

An attachment draft contains `conversation_id`, optional normalized `attachment`,
and an independent `revision`. Save supplies a local file URL; the daemon copies
the file before acknowledging the revision. Omit `file_url` to remove the saved
selection. Removal retains a revision record, so stale initial writes cannot
restore a cleared attachment. Revision conflicts use `draft_conflict`. Loading
never sends a message, and saving does not imply a send was accepted. Clients
clear only the captured attachment revision after reported Sent, Delivered, or
Displayed evidence. Text and
attachment revisions are separate, so saving a caption does not overwrite a
file selection. Workspace draft conversation IDs include attachment-only drafts.

A workspace contains `location`, `draft_conversations`, and `persistent`. A
location contains `conversation_id` and optional `message_id` in that same
conversation. A draft contains `conversation_id`, `text`, optional `reply_to`,
`revision`, and optional daemon-owned `pending_operation`. Save using the revision returned by the last load or save. An
empty draft with no reply removes the row and returns revision 0. A conflicting
revision returns `draft_conflict`; clients must reconcile rather than retrying
the stale mutation. Nonempty drafts have monotonically assigned revisions.

`messages.submit_draft` validates the text/reply command, records an outgoing
operation, and binds the submitted draft in private workspace storage before
forwarding the command. The submitted revision must match the current draft.
A bound draft rejects saves and further draft submissions, including stale
submissions from another window. Loading the draft restores the association
without replaying it. The ordinary `messages.send` API remains available for
clients that manage their own drafts.

`messages.resolve_draft_send` requires the complete current bound draft. A
reported Sent, Delivered, or Displayed outcome removes its captured text and
reply. Rejected or failed sends release the binding and retain the text/reply
for an explicit retry. Unknown or missing operations release it only for manual
review; clients must never retry those automatically. Submitting and Accepted
operations cannot be resolved. Resolution itself does not send a message.


The Rust subscription object now retains the initial snapshot's
`messaging_accounts`, `conversations`, `typing_states`, and `read_states` for
clients to build their first view. This changes no existing snapshot wire fields.

`messages.previews` returns the last saved message in each requested conversation.
It reads only the local archive, including for offline accounts. Conversations
without saved messages are omitted. Duplicate conversation IDs are collapsed.
Unknown accounts and invalid IDs reject the batch. The response can span frames;
collect until `done: true` under the normal aggregate limits. It does not fetch
provider history, mark conversations read, or claim that saved history is complete.

## Recent transfer activity

`transfers.list` returns `transfer_activity` with `activities`, or bounded
`transfer_activity_chunk` frames with `activities` and a terminal `done: true`.
Use the normal aggregate and line limits. The daemon retains at most 128 records
for its current session, newest first. Records contain a daemon-local `sequence`,
`observed_at_ms`, and an `event` tagged by `kind`: `received` with normalized
`share`, `progress` with normalized `progress`, `result` with normalized
`result`, or `download` with normalized `download`. Progress and results coalesce by device and transfer ID. A late
progress event cannot replace a retained terminal result.

Activity records represent observations. They do not fabricate a result from
command acceptance or socket byte progress. Accepted shares without later
observations may be absent. Clients read after subscribing to share events,
refetch after relevant events and replacement snapshots, and coalesce concurrent
refresh requests. `Client::subscribe_control_center` enables share and media
events without messaging history. Forgetting a device clears its activity;
daemon restart clears the full list. Received files are never deleted by this
metadata cleanup.


## Requested phone files

`files.download` takes `device_id` and a relative `path` under the file root shared
by that device. The device must be connected, paired, and advertise the normalized
`file_download` capability. The native adapter learns support from the current
phone connection; saved pairing records never imply support.

The response and subscribed event are `file_download` with a `download` record.
It contains `device_id`, `transfer_id`, `source_path`, `revision`, `kind`,
`progress`, `controls`, and `state`. `kind` defaults to `file`; `directory` requires
`file_management`. State `status` is `queued`, `preparing` with actual
`processed_bytes` and optional `total_bytes`, `receiving`, `verifying`, `paused`
with `reason`, `cancelled`, `completed` with a local `path`, or `failed` with
`reason`. Queue acceptance never proves transfer or storage. Only an atomic save
produces `completed`. `source_changed` rejects a different source version.

`progress` contains `received_bytes`, optional `total_bytes`, `completed_files`,
and optional `total_files`. `controls` advertises valid `cancel` and `resume`
actions. `files.control` takes `device_id`, `transfer_id`, and `control`,
and returns the updated `file_download`. Resume creates a fresh backend attempt
under the same public transfer ID. It never replays automatically.

`files.list` takes `device_id`, relative `path`, and optional opaque `cursor`.
Its `filesystem_queued` response contains `request_id`; the subscribed
`filesystem` result must match that ID, device, and path. `cursor_next` indicates
another page. Cursors expire and are tied to the original storage permission
scope. Clients must discard them after switching devices or folders.

Download events require `shares:true`. `transfers.list` retains them as
`event.kind:download` with `download`. Coalesce by device and transfer ID; ignore
lower revisions and preserve terminal state. Legacy records default to revision
zero. Active records survive terminal-history eviction in the bounded session
log. Daemon restart restores unfinished download checkpoints as paused, without
replaying requests. Completed session history is not durable.

The optional native `hello.files_v2` negotiation enables paged listings and
chunked read-only downloads. Phone source hashing, positioned reads, cancellation,
source-version validation, and local hash verification stay below normalized
IPC. Limits are 8 GiB per file, 10,000 folder entries, 32 unfinished jobs, two
active jobs, and 16 GiB of reserved partial data. Legacy peers keep the older
single-file path described below.

The native wire uses an optional `hello.file_downloads` boolean and
`file_request` with protocol 1, transfer ID, and relative path. Android returns
an existing `share_file` header and original bytes with the same transfer ID,
or `file_request_failure` before the header. The existing mutual TLS, filename,
size, timeout, quota, and atomic-receive rules apply. The phone resolves exact
names inside its persisted system folder grant, or inside mounted shared-storage
volumes when all-files sharing is enabled and Android grants access. Broad mode
exposes volume labels at the relative root. Local paths use read-only held
directory descriptors and reject symlinks at every segment. Document-provider
identifiers, tree URIs, and Android absolute paths never enter normalized IPC. New folder failures echo `path` so a
failure can be attached to the requested location.


## Phone volume and call audio

`phone.audio.subscribe` opts a connection into phone volume and call-routing
updates and returns current `call_audio_controls`. Older subscriptions do not
receive these new event types. Read `volume.list` for initial levels. After
subscription lag, the daemon sends replacement volume and routing state.

`volume.list` returns `device_volumes` with observed stream states. `volume.refresh`
takes `device_id`; `volume.set` takes `device_id`, `kind`, and integer `level`.
Refresh returns command acceptance; set returns `device_volume_queued` with
`request_id`. The phone must advertise
`volume_control`. Stream kinds are `media`, `ringtone`, `alarm`, and `call`.
Each reports `min_level`, `max_level`, `level`, `muted`, and `adjustable`.
`device_volume` events update actual levels. `device_volume_result` matches the
request ID with acceptance or failure, without synthesizing a changed level.
Disconnect or expiry yields `unconfirmed` for unresolved commands.

`calls.audio.controls` returns `call_audio_controls` with its revision, observed
choices, current route, and optional notice. `calls.audio.configure` takes
`expected_revision` and optional `selection`; null disarms the route. Selection
contains its expected revision, `device_id`, and opaque `endpoint_id`, `input_id`,
and `output_id`. Stale revisions fail. The daemon owns route lifetimes and emits
updated `call_audio_controls`. States are `waiting_for_call`, `routing` with
`duplex_running`, and `failed` with an explanation. Backend card names, Bluetooth
addresses, and PipeWire node names are not public identifiers. Routing does not
prove a physical call is audible.

## Local device names

`devices.rename` takes `device_id` and `name` for a paired native device. Names
are trimmed and accept up to 80 Unicode characters without control characters.
An empty name removes the nickname. The daemon persists the setting before
returning `device_updated` and publishing the updated device. The device can be
offline. This affects the desktop's display name; it does not rename the phone
or change its identity, trust, or pairing. Reconnection keeps the local nickname.
