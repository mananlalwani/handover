# Google Messages sidecar

The production Google Messages adapter lives in
[handover-gmessages](https://github.com/mananlalwani/handover-gmessages) and is
licensed under AGPL-3.0-only. This MIT repository communicates with it as a
separate process. The in-tree `handover-gmessages-helper` is a loopback helper
for development and tests.

## Layout

```text
handoverctl / Quickshell
        |  Unix socket, protocol 1, messages.*
handoverd (MIT)
  owns normalized messaging state
        |  helper IPC v1: JSON lines on helper stdin/stdout
adapter or loopback helper
  credentials, pairing, relay, media
```

The processes communicate over stdin/stdout and do not use FFI. The wire types
are defined in [`contract.rs`](../crates/handover-gmessages/src/contract.rs),
with `HELPER_PROTOCOL = 1`. Clients see `handover-core` messaging types.
Google protocol objects and enums stay inside the adapter.

`crates/handover-gmessages` is framing, normalization, staging, and
supervision only. Do not vendor `libgm`, `gmproto`, emoji tables, key
derivation constants, or endpoint lists here.

## Commands and events

Daemon → helper: `hello`, `login` (base64 bundle on the pipe, never argv),
`logout`, `list_conversations`, `fetch_history`, `send_text`, `send_media`,
`react`, `mark_read`, `typing` (start only; there is no stop),
`delete_message`, `open_conversation`, `sync`, `shutdown`.

Helper → daemon: `hello`, `account`, `account_removed`, `pairing` (opaque
prompt), `conversations`, `conversation_removed`, `messages`,
`message_removed`, `status`, `typing`, `read`, `command_result`, `error`.

Unknown protocol versions fail the handshake. Lines over 1 MiB and bad JSON
are dropped without echoing content. Unknown status tokens and capability
names are rejected, not coerced. There are no capabilities for edits,
membership changes, or disappearing messages.

`full: true` marks an authoritative window. The daemon inserts or updates the
supplied records and removes records missing from that window. `full: false`
merges records without removing missing entries. Large syncs use
size-bounded chunks that share a `generation`. Only the last chunk sets
`full`; the daemon reconciles when the generation closes. Reconciling a
middle chunk would drop records that arrive later. Events with no
`generation` still reconcile on a lone `full: true`.

`command_result ok` is acceptance, not delivery. `sent` / `delivered` /
`displayed` arrive only as later `status` events.

## Fixture

The adapter checks in `adapter/testdata/helper-events.jsonl`. Regenerate it
from the adapter repository:

```sh
REGENERATE_FIXTURE=1 go test ./adapter/ -run TestContractFixtureIsCurrent
```

After a contract change, copy the updated fixture to
`crates/handover-gmessages/tests/fixtures/helper-events.jsonl` in this repository
and run the Rust contract tests. CI compares the two files when
`HANDOVER_GMESSAGES_PUBLIC` is `true`.

## Secrets and files

`handoverctl messages login <account>` reads stdin or `--from-file`. The CLI
base64-encodes the bundle locally and sends it through the local socket and
helper pipe. Keep bundles out of command arguments, logs, and crash reports.
The limits are 1 MiB per IPC line, 192 KiB for the raw CLI bundle, and 256 KiB
for the encoded helper login bundle. The daemon does not persist the bundle.

The production adapter stores one mode-0600 session file per account under
`${XDG_STATE_HOME:-~/.local/state}/handover/gmessages`. The directory uses mode
0700, and writes use atomic rename. Pairing prompts are displayed without
logging or persistence.

Helper attachment paths must sit under an approved root, be regular
non-symlink files, and stay size-bounded. Loopback uses
`.../handover/gmessages/staging`. The production adapter uses
`.../handover/gmessages/staged`. Override with
`HANDOVER_GMESSAGES_STAGING_DIR`. Before adding attachments to normalized state,
the daemon copies
each file through a no-follow descriptor into
`.../handover/gmessages/imported`. Clients never keep helper-controlled paths.

`messages logout` revokes on the helper and deletes local secrets. The daemon
drops the account on `account_removed`. Authentication failures must report
that pairing is required. On connect, `hello` lists persisted sessions so a
restarted daemon can restore its account list.

## Running the helper

The helper is optional. `HANDOVER_GMESSAGES_HELPER` selects an explicit binary.
Otherwise the daemon searches `PATH` for the loopback binary
`handover-gmessages-helper`, then the production binary `handover-gmessages`.
If neither exists, messaging stays disabled. Other backends keep working.

Reconnect backoff ranges from 1 to 60 seconds. After connecting, the daemon
syncs known accounts and requests catch-up syncs for accounts the helper
announces. It allows at most 64 in-flight requests and waits up to four minutes
for a command. Message windows hold 300 messages per conversation, and history
pages contain at most 100. Staged attachments are limited to 50 MiB, with
32 KiB streaming buffers and the same basename rules as native shares.

If the helper exits, the daemon marks its accounts disconnected and fails
pending messaging requests. Device, notification, media, and share state remain.
Clean daemon
shutdown stops the helper. An unclean kill can leave an orphan; the next
generation replaces it.

Logs use IDs, counts, and delivery states. Bodies, names, addresses,
prompts, bundles, tokens, keys, and media bytes stay out. `redact_command`
strips bundles before a command can be logged.

## Loopback and production

The loopback helper supplies one RCS direct conversation, one SMS thread, and
one RCS group. Login stores the bundle and finishes pairing on the next sync.
Sends advance through `accepted → sent → delivered → displayed`, one step per
sync. These simulated results support daemon, CLI, UI, and contract tests.

The production adapter uses the same contract against the real relay. Point
`HANDOVER_GMESSAGES_HELPER` at its binary and follow the adapter's
[pairing runbook](https://github.com/mananlalwani/handover-gmessages/blob/main/docs/pairing-runbook.md).

Capabilities are available only when the helper advertises them: listing, paged history, live updates,
SMS/MMS/RCS marks, text and attachments, DMs and groups, replies, reactions,
typing-start, read receipts, status, own deletes, reconnect catch-up, logout.

The contract does not advertise message edits, group membership changes or
renaming, or disappearing messages. Per-participant group read state requires
relay evidence. There is no persistent database of the complete message history.

The daemon does keep a bounded cache of normalized accounts, conversations,
message windows, and read state in `handover/messaging-cache.json` under the
state directory. Set `HANDOVER_MESSAGING_CACHE=0` in the daemon environment to
disable cache loading and writes. Cached accounts start disconnected and
unauthenticated until the helper reports their current state.

Workspace tests cover daemon ↔ loopback, helper-down, gaps, bundles,
isolation, and malformed lines. One live RCS pass was done on a test
account. That is not a guarantee about Google's protocol.
