# Google Messages conversation creation evidence

Research date: 2026-11-10. This inventory used repository sources only. No live account, phone, browser, or protocol mutation was used.

## Findings

Conversation creation is not supported by the native Google Messages helper. Keep it unavailable until the request and its effects have first-party evidence. The generic normalized command and helper IPC already describe an `open_conversation` operation, but those declarations do not establish that Google accepts it or how a newly created conversation is identified and reconciled.

The existing command path checks that the account is connected and authenticated, then validates a nonempty, bounded address list and account id. It has no per-conversation capability gate because the command is account-scoped. The daemon forwards the request to helper IPC. See [`messaging.rs`](../../handoverd/src/messaging.rs) (`OpenConversation` validation), [`messaging.rs`](../../crates/handover-core/src/messaging.rs) (`MessagingCommand::OpenConversation`, `validate_command`), and [`messaging.rs`](../../handoverd/src/ipc_server/messaging.rs) (`handle_open_conversation`).

The helper contract defines `OpenConversation { request_id, account, addresses }`, and the loopback helper simulates lookup/creation. The native helper returns `command_result ok: false` with `capability not available in the native helper`; its dispatcher groups `OpenConversation` with other unsupported mutations. The loopback behavior is test support, not evidence of Google behavior. See [`contract.rs`](../../crates/handover-gmessages/src/contract.rs), [`helper.rs`](../../crates/handover-gmessages/src/bin/helper.rs) (loopback `open_conversation`), and [`helper.rs`](../../crates/handover-google-messages/src/bin/helper.rs) (native command dispatch).

The prior native audit also records that `open_conversation` reports failure. It reports decoded and acknowledged conversation inventory, but does not document a first-party create request/response or a resulting conversation identity. See [`google-messages-2026-10-09.md`](google-messages-2026-10-09.md).

## Evidence present and absent

The repository has read-only observation tools. `observe_rpc.mjs` records RPC service and method, HTTP metadata, and bounded payload shape. `wire-shape.mjs` extracts protobuf field numbers, wire types, and byte lengths while omitting values. The Chrome observer README says exports omit payload values and message contents. These tools can identify a candidate method and coarse request/response structure, but their current sanitized output cannot by itself establish field meanings, address encoding, success semantics, or the conversation ID returned by a create operation. See [`observe_rpc.mjs`](../../google-messages/tools/observe_rpc.mjs), [`wire-shape.mjs`](../../google-messages/tools/chrome-observer/wire-shape.mjs), and [`chrome-observer/README.md`](../../google-messages/tools/chrome-observer/README.md).

`index_rpc_descriptors.py` can extract RPC path and request/response symbol names from a verified bootstrap capture. The repository contains the tool and synthetic tests, but no checked-in observation export or verified capture documenting conversation creation. Descriptor names alone would still not prove runtime success or reconciliation behavior. See [`index_rpc_descriptors.py`](../../google-messages/tools/index_rpc_descriptors.py) and [`test_index_rpc_descriptors.py`](../../google-messages/tests/test_index_rpc_descriptors.py).

The helper contract says an accepted command is not proof of delivery or effect, and `open` CLI documentation says to watch for a conversation event. It does not specify behavior for a duplicate/existing recipient, server rejection, timeout after submission, or a success response without a subsequent inventory event. See [`gmessages-helper.md`](../gmessages-helper.md) and [`messages.md`](../cli/messages.md).

## First-party observations still needed

Before implementing native support, collect and retain a sanitized, independently authored record of:

- The exact Google RPC method invoked by the official Messages web client when starting a new conversation, plus request and response descriptor symbols or a field-number map grounded in first-party runtime observations.
- Requests for both an already existing direct conversation and a genuinely new recipient, so lookup/reuse behavior is distinguished from creation. Include a multi-recipient case only if the UI exposes it.
- The first-party request fields that carry recipient addresses or identities, with values removed or replaced by safe synthetic values while preserving the observed structure and encoding.
- Success and rejection responses, including the returned conversation identifier or other server-issued identity. Correlate that identity with a subsequent authoritative conversation inventory/update.
- Failure behavior when the response is rejected or lost. Establish whether retrying can create duplicates and whether an accepted command can be reconciled without replay.

Current observer output is intentionally too lossy to answer all field-semantics questions. If it cannot retain a safe field map, extend the observer only to record explicitly reviewed field numbers, types, lengths, and equality/correlation facts, never credential or message values. Do not capture or commit cookies, tokens, contact details, message content, or raw authenticated payloads.

## Bounded path if evidence becomes sufficient

Keep the normalized account-scoped `OpenConversation` API. Implement only the native helper adapter and Google request/response types needed by the observed operation. Validate inputs against observed format constraints, submit once, report acceptance separately from the later effect, and publish a conversation only from the correlated server response or authoritative inventory evidence. Preserve an unknown outcome after an ambiguous submitted request and do not retry it automatically. Keep the native capability unavailable until helper behavior and reconciliation are demonstrated with contract/workspace regression coverage and a paired-account observation.

The evidence in this repository does not yet justify that implementation. The current unavailable response should remain in place.
