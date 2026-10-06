# Google Messages sidecar

The native `handover-google-messages-helper` is bundled with Handover and is
the default Google Messages client. Explicit `HANDOVER_GMESSAGES_HELPER`
overrides take precedence. Otherwise, discovery checks beside the daemon for
the native helper, then `PATH`; only installations without a native binary
fall back to the legacy helper on `PATH`. A native authentication or session
failure never silently selects the legacy relay or retries a send through it.

The optional legacy Google Messages adapter lives in
[handover-gmessages](https://github.com/mananlalwani/handover-gmessages) and is
licensed under AGPL-3.0-only. This MIT repository communicates with it as a
separate process. Two helpers live in this repository:
`handover-gmessages-helper` is a loopback helper for development and tests, and
`handover-google-messages-helper` is the in-tree helper for the independent
client.

Handover's independently authored replacement lives in
[`google-messages/`](../google-messages/README.md) and
[`crates/handover-google-messages`](../crates/handover-google-messages/src/lib.rs)
in this repository. It must not import or copy the AGPL adapter or mautrix
implementation. Its Google wire formats and authentication remain below the
normalized helper contract; public daemon, IPC, CLI, and Quickshell models stay
backend-independent.

## Layout

```text
handoverctl / Quickshell
        |  Unix socket, protocol 1, messages.*
handoverd (MIT)
  owns normalized messaging state
        |  helper IPC v1: JSON lines on helper stdin/stdout
independent client, legacy adapter, or loopback helper
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
middle chunk would drop records that arrive later. A `full: true` snapshot must carry a `generation`; events without one
are incremental updates. Explicit history responses must echo `fetch_id` to
complete their request.

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
drops the account on `account_removed`. The native helper requires a positive
phone unpair result before deleting a confirmed account's credentials; offline
or uncertain attempts retain them. Authentication failures must report
that pairing is required. On connect, `hello` lists persisted sessions so a
restarted daemon can restore its account list.

## Running the helper

The helper is optional. `HANDOVER_GMESSAGES_HELPER` selects an explicit binary.
Otherwise discovery follows the native-first order described above.
If no helper is available, messaging stays disabled. Other backends keep working.

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

Build the test helper explicitly with
`cargo build -p handover-gmessages --features loopback-test --bin handover-gmessages-helper`.
It is never discovered automatically. Use `HANDOVER_GMESSAGES_HELPER` to select
it for a test run.

The loopback helper supplies one RCS direct conversation, one SMS thread, and
one RCS group. Login stores the bundle and finishes pairing on the next sync.
Sends advance through `accepted → sent → delivered → displayed`, one step per
sync. These simulated results support daemon, CLI, UI, and contract tests.

The production adapter uses the same contract against the real relay. Point
`HANDOVER_GMESSAGES_HELPER` at its binary and follow the adapter's
[pairing runbook](https://github.com/mananlalwani/handover-gmessages/blob/main/docs/pairing-runbook.md).

## Native helper

The optional in-tree browser setup uses daemon IPC `messages.setup.begin` and
`messages.setup.end`. These control a single bounded messaging pause and carry
only an opaque lease identifier. They do not change helper IPC v1 and expose no
Google protocol material. `messaging_setup_paused` acknowledges actual helper
shutdown and reports `expires_after_seconds`; `messaging_setup_resumed` accepts
resumption without claiming that any account is online. A lease expires after
fifteen minutes. Native setup captures authentication below the provider boundary,
performs existing registration/pairing operations, and persists only through the
native session and desktop credential stores. The daemon's recovered snapshots
remain authoritative. This path does not import the legacy adapter.

`handover-google-messages-helper` is the in-tree helper for the independent
client. It is MIT and first-party, carries no AGPL source and no generated
Google protobuf definitions, and speaks the same contract v1.

```sh
cargo build -p handover-google-messages --bin handover-google-messages-helper
HANDOVER_GMESSAGES_HELPER=target/debug/handover-google-messages-helper handoverd
```

The helper owns pairing attempts and saved credentials. One receive stream per
active account carries updates, reads, and bounded sends. Accounts become online
only after authenticated phone activation. Sending capabilities require an
authenticated phone permission response.

A successful command result reports acceptance. Phone-attested status events
report sent, delivered, or displayed outcomes. The native helper uses the daemon's
outgoing operation ID as a temporary protocol identifier, allowing an exact
account and conversation match to bind the phone-assigned message ID. Message
contents and timestamps are never used to guess an identity. Late evidence can
resolve an unknown send without replaying it.

Pairing records use random persisted Handover account aliases and restricted
files under `gmessages-native/confirmed`. Google authentication is kept in desktop
Secret Service. The daemon neither interprets nor persists browser proof. A
confirmed account's logout requires a positive correlated phone unpair result;
offline, rejected, or uncertain attempts preserve its credentials.

Native media uses authenticated chunked AES-256-GCM framing and bounded service
uploads. Files are limited to 50 MiB. Redirects and automatic send retries are
forbidden. Downloaded originals are authenticated before staging; missing
originals retain attachment metadata without substituting a preview.

Helper media staging retains at most 1,024 files, 512 MiB total, for 30 days.
The daemon imports attachments into its own private cache before publishing
normalized paths. Keys and blob references remain below helper IPC.

Capabilities are available only when advertised by the selected helper. The
contract has no message edits, membership changes, or disappearing messages.
The native helper does not implement every operation supported by the legacy
adapter. See [known limitations](KNOWN_LIMITATIONS.md).

The daemon caches bounded normalized accounts, conversations, message windows,
and read state in `handover/messaging-cache.json`. Set `HANDOVER_MESSAGING_CACHE=0`
to disable cache loading and writes. Cached accounts start disconnected until
the helper reports their current state.

## Outgoing operation recovery

The daemon records text and media sends before helper submission in
`handover/outgoing-operations.json` under the state directory. This journal is
separate from the optional message-content cache. It contains normalized IDs,
operation kind, timestamps, outcome, and an optional provider message ID.
It contains no message text, captions, attachment paths, or credentials.

The journal retains up to 512 operations. When full, a new send replaces the
oldest resolved or unknown operation; it does not evict an active submission or
accepted send. If all records are active, new sends are rejected before submission.
Atomic writes use private files and directories. A journal that cannot be restored
is preserved, and new sends fail before submission until the file is repaired.

The additive helper `send_status` event carries `request_id`, `account`,
`conversation`, optional `message`, and the existing normalized `status` token.
It correlates temporary and final provider message IDs with one daemon operation.
Generic message `status` events continue to update delivery evidence once a final
message ID is known. Older helpers without `send_status` can accept sends, but
cannot correlate them with delivery evidence in outgoing-operation records.

`messages.outgoing` returns bounded chunks with a final `done` marker. Messaging
subscriptions send nonempty outgoing snapshots after the initial state snapshot
and after lag recovery, before queued live events. A state snapshot resets the
client's outgoing collection; `outgoing_operation` and
`outgoing_operation_removed` events maintain it afterward.

On daemon restart, helper disconnection, or ten minutes without evidence beyond
acceptance, unresolved operations become `unknown`. A submitted send that loses
its acknowledgement reports `send_outcome_unknown`, rather than claiming rejection.
Late provider evidence can resolve an unknown operation. No journal entry triggers
a resend. Check the conversation before explicitly sending again. Confirmed sent,
delivered, displayed, and failed outcomes survive restart; logout removes the
account's operation records.
