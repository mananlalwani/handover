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
  send now leave paging active; history and availability failures still stop it.

Regression tests cover partial-read cancellation, oversized framing, payload
redaction, upload collisions, inventory replacement, empty-inventory permissions,
history cleanup, and paging after an unrelated command error.

## Remaining limitation

Public IPC errors do not identify their originating request or conversation.
The UI still clears all history loads for `history_unavailable`,
`unknown_conversation`, and `messaging_unavailable`. These errors can concern a
different conversation or command. Fully targeted recovery requires correlated
errors in the public IPC contract and corresponding client handling.

## Verification

- `cargo fmt --all -- --check` passed.
- `cargo clippy --workspace --all-targets -- -D warnings` passed.
- `cargo test --workspace` passed, with 510 tests passing and 3 ignored.
- Browser-setup Python tests passed, with 12 tests passing.
- All four Quickshell JavaScript test files passed.
- QML lint passed for the service, example, and pages.

Android tests and builds were not run because no Android code or public wire
contract changed. Live pairing, phone mutations, Google service behavior, and
real-account media transfer remain unverified by this audit.
