# Google Messages sidecar: process boundary, contract, and operations

Status: implemented (loopback relay). The production Google relay is
operator-supplied; see §Production relay.

## Architecture

```text
handoverctl / Quickshell
        |  Unix socket, protocol 1, messages.* methods
handoverd (MIT)
  owns normalized messaging state, validates every record and command
        |  helper IPC v1: JSON lines over helper stdin/stdout
handover-gmessages-helper (MIT, separate OS process)
  owns credential bundles, pairing ceremony, relay RPCs, polling,
  recovery, media transfer
        |  (production relay only) Google companion RPCs + phone
```

No shared address space, no FFI, no shared structs beyond the coarse
contract in `crates/handover-gmessages/src/contract.rs`. No Google
protocol vocabulary (Bugle, Tachyon, UKEY2, protobuf enums) crosses the
boundary in either direction: the helper maps its relay into generic
`conversations` / `messages` / `status` / `typing` / `read` records with
opaque local ids, and Handover maps those into account-scoped normalized
types (`crates/handover-core/src/messaging.rs`).

## Why a separate process

`mautrix/gmessages` (`pkg/libgm`) is AGPL-3.0 with exceptions granted
only to Beeper and Element (see `docs/research/google-messages-rcs.md`).
Linking, importing, FFI, or copying its sources (including generated
protobuf) into Handover would place the combined work under AGPL-3.0 and
end Handover's MIT licensing. An unmodified upstream binary driven over
an arm's-length JSON protocol is a separate-program posture: Handover
stays MIT. For that reason:

* `crates/handover-gmessages` contains framing, normalization, secret
  paths, staging, and supervision only. It contains no companion-protocol
  code and no AGPL-derived material.
* The bundled `handover-gmessages-helper` binary speaks the contract with
  a `Relay` trait whose only in-repo implementation is `LoopbackRelay`
  (in-memory seed data for development and tests).
* Never vendor `libgm` sources, `gmproto` definitions, emoji tables, key
  derivation constants, or endpoint lists into this repository.

## Helper contract v1

Defined in `crates/handover-gmessages/src/contract.rs`
(`HELPER_PROTOCOL = 1`). Daemon commands: `hello`, `login` (one-way
credential bundle, base64, piped, never argv), `logout`,
`list_conversations`, `fetch_history`, `send_text`, `send_media`,
`react`, `mark_read`, `typing` (start only; upstream cannot send
typing-stop, so no stop command exists), `delete_message`,
`open_conversation`, `sync`, `shutdown`. Helper events: `hello`,
`account`, `account_removed`, `pairing` (opaque human-readable prompt),
`conversations` (with `full` reconcile flag), `conversation_removed`,
`messages` (with `full` flag and `cursor_next`), `message_removed`,
`status` (`accepted|sent|delivered|displayed|failed:<reason>`),
`typing`, `read`, `command_result` (acceptance only), `error`.

Rules both sides follow:

* Unknown helper-protocol versions fail the handshake; oversized lines
  (>1 MiB) and malformed JSON are dropped without echoing content.
* Unknown status tokens and unknown capability names are rejected, never
  coerced into known states. There are no capability names for edits,
  membership changes, or disappearing messages.
* `full: true` pages are authoritative windows: the daemon reconciles
  (upserts plus removals for records the window no longer contains).
  `full: false` pages merge.
* Acceptance (`command_result ok`, `MessageAccepted`) is never delivery;
  `Sent/Delivered/Displayed` arrive only as attested status events.

## Auth and secret storage

* The user pastes (or automation supplies) the upstream credential bundle
  once. `handoverctl messages login <account> [--from-file PATH]` reads
  it from a file or stdin and base64-encodes it locally; bundles never
  appear in argv, shell history, logs, or crash reports.
* The bundle travels CLI → daemon → helper over local sockets/pipes only
  (bounded: 64 KiB IPC line, 256 KiB helper cap). The daemon never
  persists it. The helper stores one 0600 file per account below
  `${XDG_STATE_HOME:-~/.local/state}/handover/gmessages` (0700 directory,
  temp-file + atomic rename) and confirms with account/pairing events
  that echo no secret material.
* Pairing verification (e.g. the emoji to confirm on the phone) arrives
  as an opaque `pairing` prompt: displayed to the user, never logged
  with content, never stored.
* `messages logout` revokes helper-side (remote revoke plus local secret
  deletion); the daemon drops its copy when `account_removed` arrives.
  401/403-style auth failures from the relay must surface as
  re-pair-required account state, never silent retry with stale secrets.
* On (re)connect the helper announces persisted sessions on `hello`, so
  a restarted daemon recovers accounts without a new ceremony.

## Supervision and isolation

* The helper is optional. `HANDOVER_GMESSAGES_HELPER` names an explicit
  binary, otherwise `handover-gmessages-helper` is resolved via `PATH`;
  when neither exists the subsystem stays dormant and every other
  backend keeps working.
* Bounded exponential backoff (1s…60s). After every connect the daemon
  sends `sync` per known account; unknown accounts announced by the
  helper get a catch-up `sync` on arrival.
* At most 64 in-flight requests/fetches; command waits time out after
  30s. Message windows hold 300 per conversation; history pages cap at
  100; staged attachments cap at 50 MiB with 32 KiB streaming and
  sanitized basenames (same rules as native share names).
* Helper death marks its accounts disconnected and fails pending
  requests; device, notification, media, and share state are untouched.
  Graceful daemon shutdown stops the helper; an unclean kill can orphan
  it, and the next supervisor generation replaces it.
* Logs carry ids, counts, and delivery states only. Bodies, titles,
  names, addresses, prompts, bundles, tokens, keys, and media bytes
  never enter logs. `redact_command` strips bundles before any command
  can be logged.

## Loopback relay (development)

`handover-gmessages-helper` with no upstream wired in runs
`LoopbackRelay`: one RCS direct thread (full capabilities, text plus a
real staged attachment), one SMS thread (text only), one RCS group
(three participants, group caps). Login stores the bundle and models
pairing as pending confirmation that completes on the next sync; sends
progress `accepted → sent → delivered → displayed` one stage per sync;
typing announces inbound peer typing; deletes apply to own messages
only. It exists so the full Handover surface (daemon, CLI, UI, tests)
is exercisable without Google credentials.

## Production relay

To drive real Google Messages history, an operator runs the unmodified
upstream bridge (or equivalent) plus a small out-of-tree adapter that
implements contract v1 against it (login/pairing ceremony, relay RPC
mapping, long-poll recovery, media upload/download). That adapter lives
outside this repository to preserve the license boundary. It must honor
the same rules: coarse normalized records only, no secret or body
logging, bounded queues, explicit revoke, and no invented delivery
state. Point `HANDOVER_GMESSAGES_HELPER` (or `PATH`) at it; `handoverd`
supervises it exactly like the loopback.

## Capabilities: implemented vs unsupported

Implemented where attested: account/conversation listing, paginated
history with cursor catch-up, live conversation/message updates,
SMS/MMS/RCS transport marking, text and attachment send/receive with
bounded staging, direct and group threads, open-or-create, RCS replies,
reaction add/remove, inbound typing, typing-start pings (no stop),
mark-read/read receipts, accepted/sent/delivered/displayed/failed
semantics, own-device deletes, reconnect backoff with catch-up and
window reconciliation, logout/revoke.

Deliberately absent (no backend attests them): first-class message
edits, group member add/remove/rename, disappearing messages,
per-participant group read truth derived from status-text heuristics
(group reads stay conversation-level unless the relay attests
per-participant data), and any persistent full-history database.

## Verification

* `cargo test --workspace` (includes a live daemon↔loopback-helper
  end-to-end walkthrough plus helper-down, gap, bundle, isolation, and
  malformed/oversized contract tests).
* Manual loopback pass: `messages login/accounts/conversations/history/
  send/send-file/react/unreact/typing/read/open/logout`, daemon restart
  recovery, and KDE Connect coexistence were exercised against debug
  binaries during development.
* Real Google credentials were never used: SMS/RCS behavior against the
  production relay remains the live acceptance step (see research note
  PoC), requiring a consenting test conversation.
