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

See the [IPC coverage inventory](audits/ipc-2026-11-10.md) for the current
automated evidence. This policy does not declare protocol 1 stable.

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
| `messages.workspace` | none | `messaging_workspace` with `workspace` |
| `messages.select` | `location`, or null to clear | `messaging_workspace` with `workspace` |
| `messages.draft` | `conversation_id` | `message_draft` with `draft` |
| `messages.save_draft` | `draft` | `message_draft` with saved `draft` |

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
clear only the captured attachment revision after file-send acceptance. Text and
attachment revisions are separate, so saving a caption does not overwrite a
file selection. Workspace draft conversation IDs include attachment-only drafts.

A workspace contains `location`, `draft_conversations`, and `persistent`. A
location contains `conversation_id` and optional `message_id` in that same
conversation. A draft contains `conversation_id`, `text`, optional `reply_to`,
and `revision`. Save using the revision returned by the last load or save. An
empty draft with no reply removes the row and returns revision 0. A conflicting
revision returns `draft_conflict`; clients must reconcile rather than retrying
the stale mutation. Nonempty drafts have monotonically assigned revisions.

The Rust subscription object now retains the initial snapshot's
`messaging_accounts`, `conversations`, `typing_states`, and `read_states` for
clients to build their first view. This changes no existing snapshot wire fields.

`messages.previews` returns the last saved message in each requested conversation.
It reads only the local archive, including for offline accounts. Conversations
without saved messages are omitted. Duplicate conversation IDs are collapsed.
Unknown accounts and invalid IDs reject the batch. The response can span frames;
collect until `done: true` under the normal aggregate limits. It does not fetch
provider history, mark conversations read, or claim that saved history is complete.
