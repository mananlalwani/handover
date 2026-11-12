# Reply previews and reaction attribution

The GTK client previously required the quoted original in its current window or
local archive. The native client decoded the reply target but discarded the
phone-supplied excerpt. A read-only archive inventory found 163 replies among the
10,000 most recent stored records, with 159 distinct referenced IDs absent. This measured failed ID resolution,
not whether the original message content had been downloaded.
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

## Follow-up: reply navigation used the wrong identity

Clicking a reply could report unavailable even when its original was the
preceding loaded message. The native decoder selected reply identity field 6.2,
the transport ID, while normalized message IDs retain the phone ID. The same
first-party capture's `Tr` projection distinguishes field 1 as the phone ID
`Ic` and field 2 as the transport identity. Its reply projection selects an ID
for the web client's own model. Copying that selection into Handover's phone-ID
model broke reference equality.

Reply normalization now prefers the attested phone ID in field 6.1. It preserves
the existing fallback when that ID is absent. No text, sender, adjacency, or
approximate matching establishes identity.

The native regression decodes an original and its reply in one page, then
requires the reply target to equal the original's normalized identity. It failed
before the correction and passes afterward. The GTK fixture also verifies that
clicking a loaded quote highlights the original without requesting context.

After installing the helper, three refreshed real replies changed to corrected
targets. All three matched archived originals. A read-only `messages.context`
request returned the exact corrected original. Existing cached records are
corrected when the helper refreshes their phone records; no old messages or
archive pages were deleted. Workspace formatting, strict Clippy, workspace tests,
Android assemble/lint/unit tests, QML lint, and the narrow dark GTK fixture pass.

## Follow-up: recover stale cached replies on click

The ID correction did not update every older archive record. A further check of
the selected conversation found 212 archived messages and two replies, one with
an unresolved cached target. The latest-page check in the previous follow-up
missed that older reply. Its single-message refresh returned no record.

The new GUI regression inserts a stale reply while its original remains loaded,
clicks the quote, and requires the corrected target and loaded original to
remain visible. It failed before the recovery path. A second assertion verifies
that unavailable destination context preserves the visible messages.

An explicit `messages.context` refresh now uses the stored provider page boundary
for the source reply. It fetches that page through the helper and requires the
source's exact identity in the returned page. It does not guess identity from
text or adjacency. The GTK client refreshes unresolved reply metadata, then
follows the corrected target. Destination context must contain that exact target
before replacing the current view. Ordinary archive reads retain their existing
behavior, including offline reads.

A live refresh of the previously unresolved source returned the exact reply in
2.7 seconds. Its target changed to an original already in the same archive.
A subsequent context request returned that exact original. This verifies the
older cached record that the latest-page check had missed. The actual live GUI
click was not automated; the click path is covered by the GTK fixture.

The update was installed in both the daemon and Messages app. Workspace
formatting, strict Clippy, workspace tests, Android assemble/lint/unit tests,
QML lint, JavaScript tests, and the GTK fixture passed. No real messages or
reactions were sent, and no archived messages or page boundaries were deleted.
