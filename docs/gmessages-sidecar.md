# Google Messages sidecar

Production Google Messages lives in
[handover-gmessages](https://github.com/mananlalwani/handover-gmessages)
(AGPL-3.0-only). This MIT repo talks to it as a separate process. The in-tree
`handover-gmessages-helper` is a loopback for development and tests.

This is the engineering boundary, not a legal opinion about every way the two
trees might be combined.

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

No shared address space, no FFI. The wire types are
`crates/handover-gmessages/src/contract.rs` (`HELPER_PROTOCOL = 1`). Clients
see `handover-core` messaging types. Google names (Bugle, Tachyon, UKEY2,
protobuf enums) do not cross the pipe.

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

`full: true` is an authoritative window. The daemon upserts and removes
records missing from that window. `full: false` merges. Large syncs use
size-bounded chunks that share a `generation`. Only the last chunk sets
`full`; the daemon reconciles when the generation closes. Reconciling a
middle chunk would drop records that arrive later. Events with no
`generation` still reconcile on a lone `full: true`.

`command_result ok` is acceptance, not delivery. `sent` / `delivered` /
`displayed` arrive only as later `status` events.

## Fixture

The adapter checks in `adapter/testdata/helper-events.jsonl` (`REGENERATE_FIXTURE=1
go test ./adapter/ -run TestContractFixtureIsCurrent`). This repo copies it to
`crates/handover-gmessages/tests/fixtures/helper-events.jsonl`. After a contract
change, refresh the Go fixture, copy it here, run the Rust contract test. CI
diffs the two files when `HANDOVER_GMESSAGES_PUBLIC` is set.

## Secrets and files

`handoverctl messages login <account>` reads stdin or `--from-file`. The CLI
base64-encodes locally. The bundle never appears in argv, logs, or crash
reports. Caps: 64 KiB IPC line, 192 KiB raw CLI, 256 KiB encoded helper
login. The daemon does not persist the bundle.

The production adapter stores one mode-0600 session file per account under
`${XDG_STATE_HOME:-~/.local/state}/handover/gmessages` (directory 0700, atomic
rename). Pairing prompts are shown, not logged, not stored.

Helper attachment paths must sit under an approved root, be regular
non-symlink files, and stay size-bounded. Loopback uses
`.../handover/gmessages/staging`. The production adapter uses
`.../handover/gmessages/staged`. Override with
`HANDOVER_GMESSAGES_STAGING_DIR`. Before normalized state, the daemon copies
each file through a no-follow descriptor into
`.../handover/gmessages/imported`. Clients never keep helper-controlled paths.

`messages logout` revokes on the helper and deletes local secrets. The daemon
drops the account on `account_removed`. Auth failures must become re-pair
state, not silent retry. On connect, `hello` lists persisted sessions so a
restarted daemon does not need a new ceremony.

## Running the helper

Optional. `HANDOVER_GMESSAGES_HELPER` wins. Otherwise `PATH` is searched for
`handover-gmessages-helper` (loopback) then `handover-gmessages` (production).
If neither exists, messaging stays off. Other backends keep working.

Backoff is 1s to 60s. After connect, the daemon `sync`s known accounts and
catch-up-syncs accounts the helper announces. At most 64 in-flight
requests. Command wait is 4 minutes. Windows hold 300 messages per
conversation. History pages cap at 100. Staged attachments cap at 50 MiB
with 32 KiB streaming and the same basename rules as native shares.

Helper death marks those accounts disconnected and fails pending messaging
requests. Device, notification, media, and share state stay. Clean daemon
shutdown stops the helper. An unclean kill can leave an orphan; the next
generation replaces it.

Logs use ids, counts, and delivery states. Bodies, names, addresses,
prompts, bundles, tokens, keys, and media bytes stay out. `redact_command`
strips bundles before a command can be logged.

## Loopback vs production

Loopback: one RCS DM, one SMS thread, one RCS group. Login stores the bundle
and finishes pairing on the next sync. Sends walk
`accepted → sent → delivered → displayed` one step per sync. Enough to drive
daemon, CLI, UI, and tests without Google.

Production: same contract against the real relay. Point
`HANDOVER_GMESSAGES_HELPER` at that binary. Pairing runbook lives in the
adapter repo.

Attested where the helper says so: listing, paged history, live updates,
SMS/MMS/RCS marks, text and attachments, DMs and groups, replies, reactions,
typing-start, read receipts, status, own deletes, reconnect catch-up, logout.

Not attested: message edits, group membership/rename, disappearing messages,
per-participant group reads unless the relay actually sends them, a
persistent full-history database.

Workspace tests cover daemon ↔ loopback, helper-down, gaps, bundles,
isolation, and malformed lines. One live RCS pass was done on a test
account. That is not a guarantee about Google's protocol.
