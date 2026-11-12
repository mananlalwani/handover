# Native mark-read

The Rust app could render read-state updates, but the native helper discarded
mark-read commands and withheld the capability. A capability regression failed
against that implementation. A second regression caught the helper dropping the
capability when converting native conversations to normalized helper records.

## First-party evidence

This implementation was authored from the previously captured Google public web
client module, not another implementation. The capture is
`/tmp/handover-public-bootstrap-nov07/conversation-public.js`, 2,767,426 bytes,
SHA-256 `e75370a39d88704396ab7cf776d12f9220ff1938291cfac78424f628651cc071`.
The public bootstrap capture identifies Google Messages web and the
`messagesweb.mw.en_US.CU37_ktczas.O` build. No credentials or user messages were
needed to inspect it, and no downloaded source is included in this repository.

The web client's `Dn` method sends phone action 10. It writes the conversation
identifier to field 2 and the optional message identifier to field 3. Its optional
field 4 is omitted here. The `mGa`/`nGa` declaration confirms those string fields.
The method awaits the response without interpreting a success field. The `kI`
conversation projection reads unread count from field 6 with protobuf's numeric
zero default. This also corrects the native projection treating an omitted zero
as unknown. Negative counts still fail validation.

## Behavior

The helper routes mark-read through the existing bounded worker command queue and
receive owner. The session validates identifiers and known conversation scope,
posts the encrypted command, waits for its correlated completion, acknowledges
it, and refreshes the authenticated phone inventory. Only that inventory establishes the
resulting normalized unread state. A still-positive count keeps the dot visible;
HTTP acceptance alone never clears it. The existing helper IPC contract and
public Handover models are unchanged.

The capability is exposed after the existing authenticated phone permission
response. The GTK app requests it only for a focused conversation at its newest
messages. Losing focus or reconnecting allows another request, and an immediate
request failure clears the local deduplication guard. No outgoing messages are
retried as part of this operation.

An encrypted mock-phone scenario starts with unread messages, checks action 10,
returns its correlated bodyless completion, then serves authenticated refreshed inventory. It verifies
both a cleared unread state and an inventory that remains unread. Existing
receive ownership, push publication, history, reaction, unpair, and expiry checks
remain in the same scenario. Unit fixtures verify exact request bytes and helper
capability translation.

## Live completion format

The first live attempt exposed a response-shape mismatch and left the receive
worker offline. A temporary diagnostic recorded only three byte counts after
request/action/paired-phone correlation checks. The phone's mark-read completion
had zero encrypted, plaintext, and auxiliary payload bytes. No identifiers,
credentials, message contents, or raw response bodies were recorded. The
instrumentation and its temporary metadata files were removed.

The receiver now accepts this empty completion only for action 10. Request and
phone identity checks, preemption handling, and bounds remain intact. Plaintext
and auxiliary payloads are still rejected. Other actions still require encrypted
content. The completion attests no read state; only the subsequent encrypted
inventory can do that. A regression failed against the old parser and now checks
that the exception cannot admit empty replies for inventory, capabilities,
reactions, or unpairing.

## Startup admission and verification

Live recovery also exposed an early request rejected while the helper's copy of
capabilities was still being published. Mark-read admission now requires an
online account, a known conversation, and room in its existing worker queue.
The worker still checks its current authenticated permission before posting the
request. A regression covers capability publication, account/conversation scope,
and queue saturation. This avoids trusting a stale duplicate capability copy.

Formatting, strict workspace Clippy, 633 workspace tests with 7 ignored, Android
assembly/lint/unit tests, QML lint, 49 JavaScript tests, and regular light/narrow
dark GUI scenarios passed. The encrypted mock verifies the unread-to-read
transition and a response that remains unread. The live app subsequently
received a phone-confirmed read state and message marker for the open native
chat, with the account connected and authenticated. That chat was already read
on the phone, so a live unread-to-read transition remains unverified.

The tested app and helper were installed. Normal app closure preserved drafts;
later helper reloads retained the open window. Pairing was retained, the desktop
bridge activation state was preserved, and no text or media messages were sent.
The initial failed live attempt was recovered after the bodyless completion fix.
