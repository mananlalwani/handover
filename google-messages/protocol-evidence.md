# Google Messages protocol evidence

The native client is independently authored from Google's public web client and
first-party observations. No downloaded client code or generated definitions are
included in this repository. This record preserves the source builds, field
meanings, and observed limits behind the implementation.

## Mark-read and replies

The public bootstrap capture from November 7, 2026 identifies build
`messagesweb.mw.en_US.CU37_ktczas.O`. Its `conversation-public.js` was 2,767,426
bytes, with SHA-256
`e75370a39d88704396ab7cf776d12f9220ff1938291cfac78424f628651cc071`.

The web client's `Dn` sends phone action 10 with the conversation identifier in
field 2 and an optional message identifier in field 3. Handover omits optional
field 4. The `mGa`/`nGa` declaration confirms the string fields. `Dn` awaits a
response without interpreting a success field. The `kI` conversation projection
reads unread count from field 6, using protobuf's zero default.

A correlated live action-10 completion had zero encrypted, plaintext, and
auxiliary payload bytes. The receiver accepts that empty completion only for
actions that explicitly support it. It supplies no read-state evidence; a
subsequent authenticated inventory determines whether the conversation remains
unread. A live check confirmed a read-state response for an already-read chat.
A physical-phone unread-to-read transition was not verified.

The `r8a` reply projection reads reply field 3, body field 2, then text field 1.
Handover preserves this excerpt as `reply_preview`, without creating an original
message from it. The `Tr` identity projection distinguishes field 1, the phone
ID `Ic`, from field 2, the transport ID. Handover's normalized message identity
uses the phone ID, so reply targets prefer field 6.1 over field 6.2.

The `lda` lookup requests phone action 36 with message field 1 and optional part
field 2. Its response projects a full message from field 1. Handover reuses that
lookup without a part for original-message retrieval. A correlated live response
had no payload, so successful direct retrieval remains covered by synthetic
transport tests rather than a live observation.

Older cached replies can retain transport IDs. An explicit context refresh uses
the source reply's stored provider page boundary and requires its exact identity
in the response. It then follows the corrected target. A read-only live refresh
returned a corrected reply and its exact archived original. Text, adjacency, and
timestamps never establish message identity.

Reaction actors are normalized from authenticated conversation participant roles.
The private alias index is bounded and scoped to account and conversation.
Unknown ownership cannot authorize removal of a reaction.

## Multiple attachments

A public bootstrap capture on November 14, 2026 used build
`messagesweb.mw.en_US.uwar1xJBL1A.O`. Its bundle SHA-256 was
`8ad7b249e55fb95324b08afec35881c15180be1a409333e12ec627f9440b368b`.
No account, cookies, or messages were accessed.

The `oA.XP` selector enumerates and orders a logical message's parts. The `G$a`
builder maps them to repeated content field 10 through `Kv`. Each media part
retains its filename, MIME type, upload identity, size, and key; text is a
separate part. The Bugle backend wraps the result once in `Vv` and submits phone
action 3.

A local synthetic harness evaluated the extracted builder with two image entries
and one caption, using inert field setters. It retained three parts in one
content object without making network requests. This establishes collection
behavior, not phone acceptance or delivery.

Handover uploads files sequentially and constructs one send request after every
upload succeeds. Ten attachments is Handover's limit, not an observed provider
maximum. Real-account multi-file acceptance and recipient delivery were not
verified by this investigation.

## Conversation creation

The normalized `OpenConversation` command and helper IPC exist, but the native
helper rejects the operation. The available public-client captures and observer
output do not establish the request fields, completion semantics, or how the
result maps to the conversation inventory. Native creation remains unavailable.

Before implementation, a first-party observation must establish:

- Inputs for one-to-one and group creation, including participant address format.
- The correlated completion, errors, and behavior when a thread already exists.
- The resulting conversation identity and inventory reconciliation.
- Phone capability gating and SMS/RCS selection.

Record reviewed field numbers, types, lengths, and correlation facts. Do not
retain authenticated payloads, credentials, message text, or contact details.
Acceptance must remain distinct from a later authoritative conversation update;
an uncertain submitted request must not be replayed.

See [incoming typing](../docs/gmessages-typing.md) for the separate RCS receiver.
