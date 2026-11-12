# Reply previews and reaction attribution

The GTK client previously required the quoted original in its current window or
local archive. The native client decoded the reply target but discarded the
phone-supplied excerpt. A read-only archive inventory found 163 replies among the
10,000 most recent stored records, with 159 distinct referenced originals absent.
No message contents, contact identities, or provider identifiers were recorded.

## First-party evidence

The existing public Google Messages bootstrap capture uses build
`messagesweb.mw.en_US.CU37_ktczas.O`. Its `conversation-public.js` SHA-256 is
`e75370a39d88704396ab7cf776d12f9220ff1938291cfac78424f628651cc071`.

The first-party `r8a` reply projection reads the reply's field 3, then its body
field 2 and text field 1. The native client now preserves that excerpt as optional,
backend-independent `reply_preview` text. This text belongs to the reply, and is
never published as a fabricated original message.

The same capture's `lda` requests action 36 with message field 1 and optional
part field 2. Its response projects a full message from field 1. Handover already
used that request for attachment enrichment. The new targeted lookup reuses it
without a part. An observed correlated reply from the paired phone had zero
bytes in its encrypted, plaintext, and auxiliary fields. That supplies no
original content. Parsing this completion as an encrypted data reply previously
ended receive; it now reports absence and leaves the connection available.
Only a decoded, validated full record with the exact requested account,
conversation, and message identity may enter the archive.

No AGPL implementation or generated definitions were used.

## Behavior

Reaction actor aliases come from authenticated conversation participant roles.
The private index is bounded and scoped to account and conversation. Multiple
attested local SIM identities normalize to one self actor. Unknown actors remain
unknown and cannot unlock the remove-own-reaction action.

Reaction chips open participant lists using the existing contact-name and photo
lookup. The GUI fixture checks another participant's name and photo, the `You`
label, and the ownership guard for removal.

Reply labels prefer a known original. Otherwise they show the phone's excerpt,
then a loading or unavailable label when neither is present. Targeted original
lookups coalesce, time out, and leave the current history window unchanged.
Old archive entries acquire excerpts when the phone supplies refreshed records.

## Validation

Read-only checks against the paired phone confirmed four refreshed replies with
preserved excerpts. A missing-original lookup completed in 0.4 seconds with an
unavailable result, and the account remained connected afterward. Archived
reaction data included 33 actors matching attested self participants after
normalization. No messages or reactions were sent for verification.

Successful full-original retrieval is covered by the encrypted synthetic phone
and daemon integration tests. It was not observed against the paired phone in
this session. Older archive records without excerpts can still show unavailable
until refreshed data or an original becomes available.

Checks cover workspace formatting, strict Clippy, workspace tests, Android
assemble/lint/unit tests, QML lint, 49 JavaScript tests, and the GTK integration
fixture at regular width and at 760 pixels in dark mode. The GTK fixture checks
immediate phone excerpts, later original resolution, unavailable originals,
reaction participant photos, self labels, and ownership guards. Tests requiring
live audio hardware remain ignored by the workspace suite.
