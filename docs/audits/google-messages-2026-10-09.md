# Google Messages audit, 2026-10-09

The audit covered the native client, helper framing and supervision, attachment
staging, daemon history and outgoing-operation integration, browser setup, and
Messages UI history handling. Protocol behavior was assessed against the in-tree
contract, native sources, and existing independently authored fixtures. Verification
used automated tests and local mock services. No live Google account operations
were performed.

## Fixed findings

- Helper reads lost already-consumed bytes when an outbound command cancelled a
  partial read. Persistent framing state now survives cancellation, including
  oversized-record discard. An unterminated record beyond the discard limit
  terminates the session and restarts the helper.
- Command debug logging included text, captions, paths, reactions, and recipient
  addresses. Command summaries now allow only command names and request IDs.
- JSON decoding errors could quote rejected payload values. Error reports now
  retain only fixed text and parser positions. Unknown status and capability
  tokens are also excluded from errors and status logs.
- Concurrent attachment uploads could overwrite shared temporary or destination
  files. Each upload now uses a private temporary file and exclusive publication.
  Type and size checks apply to the same opened descriptor used for copying.
- Worker conversation inventories retained records absent from a refreshed
  inventory. Refreshes now replace the worker inventory; live updates merge.
  Helper IPC conversation pages remain incremental and make no deletion claim.
- Sending permission was inferred from existing conversation capabilities when
  processing pushes. An empty inventory lost an authenticated permission result,
  leaving a newly announced conversation unable to send. Permission now has its
  own session state and applies consistently to refreshes and pushes.
- Interrupted history fetches left page-floor entries behind. Timeout and helper
  failure cleanup now remove them.
- Unrelated command errors stopped UI history loads. Errors such as a rejected
  send now leave paging active. History and availability failures name the
  conversation they address, and the UI stops only that conversation's load.

Regression tests cover partial-read cancellation, oversized framing, payload
redaction, upload collisions, inventory replacement, empty-inventory permissions,
history cleanup, paging after an unrelated command error, and history recovery
scoped to the conversation named on the error.

## Verification

- `cargo fmt --all -- --check` passed.
- `cargo clippy --workspace --all-targets -- -D warnings` passed.
- `cargo test --workspace` passed, with 511 tests passing and 3 ignored.
- Browser-setup Python tests were not re-run; that code did not change.
- All four Quickshell JavaScript test files passed.
- QML lint passed for the service, example, and pages.

The desktop IPC error payload now carries an optional `conversation_id`.
Android tests and builds were not run because no Android code changed. Live
pairing, phone mutations, Google service behavior, and real-account media
transfer remain unverified.

## Follow-up fixes

The open findings from the second pass are fixed in the working tree.

- A completed native inventory is one generation. The last chunk is `full: true`,
  including an empty inventory. Live conversation updates stay incremental.
- Helper `error` events carry an optional `fetch_id`. Only that history read
  fails. Other helper errors leave in-flight history alone.
- Catch-up requests 50-message pages, the native maximum, and continues while
  the provider cursor advances, up to 300 messages or the cached window.
- A history page made only of system notices is followed, up to eight hops,
  before it is published. The history reply is acknowledged after that
  publication. Direct history reads acknowledge after the page is returned.
- Registration renewal drains a queued command before it replaces the worker.
- Logout, session end, and a lost receive session complete the affected history
  read with `native history interrupted` instead of waiting out the command
  timeout.
- Authoritative conversation removal and window pruning delete the matching
  archive rows.
- Quickshell account removal clears that account's messages, cursors, and
  history loads.
- A confirmed phone unpair deletes local session records even when Secret
  Service cleanup fails, and reports that failure afterward. An unconfirmed
  logout deletes the Secret Service item only after session removal succeeds.
  The loopback helper reports an error and keeps the account when credential
  deletion fails.

Conversation-list pages are still acknowledged as each page is decoded so the
next page can be requested. That acknowledgement happens before the finished
inventory is published.

## What held up

- Sends are not replayed after reconnect, resume, logout, or worker death.
  Delivery evidence is bound by message id and the daemon operation id. Body
  text and timestamps are not used as identity.
- Send permission comes from the phone permission result and is kept separately
  from the conversation inventory. `delete_message` and `open_conversation`
  report failure. `mark_read` and outgoing typing report no success.
- Invalid credentials, HTTP 401 and 403, API-key failures, and session
  preemption are not retried. Network failures, timeouts, rate limits,
  temporary RPC failures, interrupted receive streams, and a locked keyring
  are retried with a 60-second cap.
- Browser proofs and login bundles travel on stdin and the helper pipe. The
  daemon does not persist the bundle. Session files use mode `0700`/`0600`,
  `O_NOFOLLOW`, owner checks, and atomic replace. Media upload and download
  refuse redirects and proxies, cap bodies while reading, and stage through
  no-follow descriptors.
- Outgoing journal records hold ids, kind, and outcome. Restart and helper
  death turn submitting and accepted operations into `unknown` and do not
  resend. Eviction skips those active states.
- Middle sync chunks stay buffered. The daemon prunes once, on the closing
  `full: true` chunk, and drops `full: true` when no generation is set.
- Incoming typing is published before its acknowledgement, and only for one
  direct RCS conversation whose peer address matches exactly.

The follow-up was checked with `cargo fmt --all -- --check`, Clippy on
`handover-google-messages`, `handover-gmessages`, and `handoverd`, the tests for
those three packages, and the Quickshell history script. Android builds were
not run. No live Google account was used. The full workspace suite was not
re-run.
