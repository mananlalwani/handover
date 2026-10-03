# Pairing investigation

This note records observations of Google's own bootstrap script captured on
2026-10-02. The source hash and capture process are in
[bootstrap-evidence.md](bootstrap-evidence.md). No mautrix implementation or
protocol definition was used.

## What the bootstrap establishes

- The source names a QR authentication provider and includes an account sign-in
  RPC. Google's consumer documentation describes the Google-account pairing
  flow. These observations do not establish the account provider's implementation.
- Its registration service includes an account sign-in method; its pairing
  service includes registration, refresh, revocation, and web-key operations.
- Compiled RPC descriptors identify separate request and response classes.
  Their minified JavaScript names are unstable implementation identifiers,
  not a protocol schema or wire-field names.
- The client explicitly selects a JSPB format in several RPC clients. Other
  descriptors contain binary serializers. This does not establish which form
  applies to the account-based pairing requests we need.
- The source includes several standard cryptographic primitive names. Their
  presence alone does not establish the handshake, key derivation, encryption
  format, or authenticated-data construction. None has been implemented yet.

## Anonymous network observation

An isolated, unsigned-in browser fetched the current page. It made a separate
Google account-menu RPC, but no instant-messaging registration or pairing RPC
was observed during the bounded startup trace. Google-account authentication is
therefore the next observation checkpoint, not a step we can infer from the
anonymous page alone.

## Authenticated read-only observation

The user approved using the current account with a brief scheduled interruption
of the existing relay. Do not import the old adapter's persisted session or use
its protocol implementation to fill missing details. Pause the old relay before
activating the test browser, and restore it after the test session closes.

The observer may retain service/method names, HTTP status and content type, and
bounded structural body shapes. It must discard actual body values, cookies,
headers, account identifiers, message content, and cryptographic material. Those
values must never appear in stdout, captured fixtures, Git, or debug logs.

A successful Google web pairing is protocol evidence, not proof that Handover's
independent pairing implementation works. The acceptance checkpoint still needs
Handover-authored registration, pairing, session restoration, and a read-only
conversation/history fetch without executing Google's application bundle.

## Observed pre-authentication pairing traffic

The approved browser trace reached these RPCs on 2026-10-02 before the
Google-account sign-in attempt was rejected:

| RPC | POST status | Request content type | Response content type |
| --- | --- | --- | --- |
| Pairing/RegisterPhoneRelay | 200 | application/x-protobuf | application/x-protobuf |
| Pairing/RefreshPhoneRelay | 200 | application/x-protobuf | application/x-protobuf |
| Messaging/ReceiveMessages | 200 | application/json+protobuf | application/json+protobuf; charset=UTF-8 |

The receive request was a four-position JSON array. Its first position was a
seven-position array; its fourth position was an empty array. Values were
discarded. The observer cannot decode the binary pairing bodies, and any byte
counts from CDP text representations must not be treated as binary wire sizes.
The receive response remained a live stream during this observation, so this
trace does not supply a complete receive-response shape.

Google rejected sign-in with "This browser or app may not be secure." No
account authentication, phone confirmation, conversation list, or history fetch
was verified. The existing relay was restored immediately after the reported
failure. The browser rejection reason has not been isolated.

These observations resolve the transport-format question for the observed
requests. They do not establish protobuf field meanings, authentication token
sources, key derivation, or encrypted envelope construction.

## Normal-browser follow-up

The normal Chrome profile became available through the browser connector. The
user completed Google's passkey confirmation, and the Google Messages page
displayed its conversation list. No messages were sent and no conversation
contents were saved as fixtures.

This verifies access through Google's application in normal Chrome. The browser
connector provides page interaction, but no network interception capability.
Chrome was not started with a remote-debugging endpoint, so the standalone CDP
observer could not attach to this session. Authenticated RPC schemas and
independent-client pairing remain unverified.

## Authenticated normal-Chrome trace

The local Chrome observer was loaded and exercised by the user. After the
existing helper was requested to pause, the user refreshed the signed-in Google Messages
page, exported 215 sanitized records, stopped the observer, and closed the test
page. The original helper was restored and its account reported online.

All captured POST responses had HTTP status 200 and content type
`application/json+protobuf`. HTTP status alone does not establish an RPC result,
message delivery, or any other effect.

| RPC | Requests | Request array positions | Completed response array positions |
| --- | ---: | ---: | ---: |
| Registration/SignInGaia | 2 | 4 | 3 |
| Registration/ListIdentities | 1 | 2 | 2 |
| Registration/LookupRegistered | 1 | 7 | 4 |
| Messaging/ReceiveMessages | 2 | 4 | Not captured |
| Messaging/PullMessages | 2 | 2 | 3 |
| Messaging/AckMessages | 17 | 2 | 1 |
| Messaging/SendMessage | 48 | 9 | 2 |

One SendMessage request had no matching completed response before export. The
receive streams stayed open, so the observer did not record their body shapes.
The export retained structural metadata only and remains outside Git.

The normal-Chrome public script shows that SendMessage carries an opaque payload
inside a transport envelope. In this build, the send helper is at character
offset 1061950, the envelope class at 703962, and the request descriptor at
931901. Its envelope setter places bytes in field 12; the send helper assigns a
request identifier and transport metadata before invoking the RPC. This is not
a parsed chat-send operation. The automatic traffic during refresh does not
prove that a user chat message was sent, and the payload meanings were not
captured.

This trace establishes the authenticated transport format and useful structural
checks for future original-client work. It does not establish cookie-based
authentication construction, protobuf field semantics, pairing cryptography,
session restoration, or conversation/history decoding by Handover's own client.

## Original read-only authentication probe

The source indexed in the normal-browser follow-up provides a small first
request that can be implemented without the adapter. `Z4a` at character offset
1068677 builds SignInGaia; `$4a` at 1068864 calls it with mode 1 to list registered
sources. The account provider's registration path uses mode 0 instead. The
probe implements only mode 1.

The request header uses a fresh request identifier in field 1 and the observed
client label in field 3. The device wrapper's field 1 contains a type-3 device
identifier. The device class and factory are at 694800, the wrapper setter at
695172, and the web identifier constructor at 1071318. All identifiers in the
probe are newly generated. Optional client-version metadata is omitted until
its contract has been verified.

The response class `mPa` at 771914 exposes field 3 as a `YC` container. `$4a`
reads its repeated field 3 as registered sources and defaults absent fields to
an empty list. The probe validates this bounded array structure and discards
source records after counting them. It does not expose their identifiers.

The original HTTP transport uses JSON+protobuf and the public transport label
`grpc-web-javascript/0.1`, as shown by `J_a` near 948933. The native probe receives
a one-use observed authorization proof through Chrome Native Messaging and
makes its own request. This initial cookie-free mode never receives cookies or replays captured bodies,
imports adapter state, or executes Google's bundle in Rust. Whether this
cookie-free request authenticates remains a live-test question.

The first native live test returned `http_error`, HTTP 401, reported by the user.
The request reached Google but its authorization was rejected. This does not
isolate the cause. The user approved a separate one-time comparison including
only the Cookie header attached to the matched SignInGaia request. Those values
must remain in transient process memory and never become fixtures or account
storage. This comparison is not yet live-tested.

The test-control check found that the runtime drop-in named
`90-protocol-readonly-test.conf` sorted before the user's existing
`override.conf`; its helper setting was overridden. Renaming it to
`zz-protocol-readonly-test.conf` and checking the effective systemd environment
and zero production helper processes verified the corrected pause. Earlier
pause claims without those checks are unverified. The test overrides were
removed and the production account reported online after the native result.

The approved service-cookie comparison returned HTTP 400, reported by the user.
The status changed from the previous cookie-free 401, but these were separate
requests with fresh proofs. This does not by itself establish successful
authentication or prove that cookies caused the change. The relay was restored
and the test page closed afterward.

The sanitized authenticated request shape has seven header positions. Its field
7 is a nine-position client-info array with numeric fields 3, 4, 5, 7, and 9.
Google's `GH` builder at 956120 sets fields 7 and 9 to 4 and 6, and obtains the
three version components from the public build label. `OPa` at 777875 parses that
label. The original anonymous capture declares
`comms-messages.web-server_20260930.02_p0`. A fresh anonymous read on 2026-10-02
returned `comms-messages.web-server_20261001.02_p0`, giving components 20261001,
2, and 0 for this test. The current public script, SHA-256
`18f7b5199eb7f7c13474f312876464ad960a1438616e6afc3ddd98efdb65870b`,
also confirms client-info fields 7 and 9 are still 4 and 6, and version fields
3, 4, and 5 use the three build components. Its minified symbols have changed.
The experimental probe now includes this observed metadata. This is a pinned
wire-version test, not automatic version discovery or a claim of compatibility
with future Google builds. The device identifier format is unchanged for this
comparison, so the metadata block is the only request-body change.

For errors, the probe can now return only a fixed `google.rpc.Code` category
from Google's documented [HTTP JSON error representation](https://google.aip.dev/193).
Error bodies are bounded to 16 KiB with a one-second read limit. Unknown status
strings, messages, and details are discarded. This diagnostic does not expose
Google's free-form error text or establish that this particular service follows
the documented representation. The metadata change is not yet live-tested.

The metadata comparison still returned HTTP 400 with no projected RPC category.
This did not resolve the rejection. Reviewing the public transport shows that
its JSON-protobuf error decoder reads RpcStatus as an array: code in field 1,
message in field 2, repeated details in field 3. In the normal-browser source,
`r_a` at 933437 invokes `xKa`, which decodes the `wKa` class at 692543. The native
probe initially supported only the AIP-193 object representation. It now also
accepts bounded JSON-protobuf error arrays and projects only canonical numeric
error codes 1 through 16. Code 0, unknown codes, malformed message/detail shapes,
and free-form values never become diagnostics. Matching the public transport,
this error decoding does not depend on the response media type. Successful RPC
responses still require their expected JSON-protobuf media type and shape.

Google's fresh-device fallback uses a hyphenated UUID, so the probe's fresh
device-ID length remains supported by the observed source. No device-ID change
was made based only on the shorter stored identifier in the sanitized capture.
The next test changes only error decoding, without changing the request body.

The array-error decoder was live-tested with the approved cookie comparison.
The user reported HTTP 400 with RPC INVALID_ARGUMENT. This confirms that the
fixed-category projection works against the live endpoint, but does not prove
authentication succeeded or identify the rejected argument. The relay was
restored and its account reported online.

Reviewing the sanitized successful browser response also found that YC has four
positions, including field 4 beside the source list in field 3. The success
decoder's former three-position limit rejected this observed shape. A synthetic
fixture preserving that topology failed before increasing the bound to four
and passed afterward. Adjacent identity records and field 4 remain discarded.
This fixes a separate decoding defect and does not resolve the live HTTP 400.

The next controlled request test changes only the fresh device-ID suffix from
a hyphenated UUID of length 36 to UUID hexadecimal of length 32. The resulting
45-character identifier matches the working browser capture's length. Google's
public builder supports both a stored 32-character suffix and a 36-character
fallback, so the longer form has not been proven invalid. The experiment keeps
the identifier fresh rather than borrowing the browser's device identity.
Request mode, headers, client metadata, and request-ID format stay unchanged.
The shorter form has not yet been live-tested.

The compact-device-ID comparison also returned HTTP 400 / INVALID_ARGUMENT,
reported by the user. Matching the working identifier length did not resolve
the failure; the experiment does not identify its cause. The test page closed
and the restored production account reported online.

The next diagnostic projects eight recognized infrastructure reasons from a
structured google.rpc.ErrorInfo object with domain googleapis.com. Google's
[public ErrorReason definitions](https://docs.cloud.google.com/php/docs/reference/common-protos/latest/Api.ErrorReason)
distinguish invalid API keys, API restrictions, HTTP referrer restrictions,
IP restrictions, Android or iOS restrictions, invalid consumers, and disabled
services. Only those fixed enum strings can leave the probe. Free-form messages,
metadata, unknown types, domains, and reasons remain discarded. More than 16
details and conflicting recognized reasons produce no reason. This supports
structured JSON ErrorInfo objects in either supported status representation;
opaque protobuf Any payloads are not decoded. Neither this service's use of
ErrorInfo nor any particular infrastructure rejection is established yet.
Request bytes are unchanged for this diagnostic.

The structured-object diagnostic also returned HTTP 400 / INVALID_ARGUMENT
without a recognized reason, reported by the user. This does not prove that
Google omitted details: the reader did not yet decode JSPB Any representations.
The production relay was restored online and the diagnostic tab closed.

The public browser source defines its Any class at 290226, with type URL in
field 1 and an embedded array or bytes in field 2. The diagnostic now accepts
that bounded two-position representation for the exact ErrorInfo type URL.
An embedded ErrorInfo array projects only positions 1 and 2; a base64 payload
uses prost to decode only reason field 1 and domain field 2 from Google's
[public ErrorInfo schema](https://github.com/googleapis/googleapis/blob/master/google/rpc/error_details.proto).
The partial Rust message is independently authored; metadata field 3 is skipped
by the library. Its Debug output is redacted. No AGPL adapter source or generated
protocol files were used. Malformed encodings, oversized values, unknown reasons,
and unrelated domains produce no reason. The synthetic JSPB regression failed
before this change and passed afterward. Live coverage remains unverified.
Neither the request nor the extension changed for this decoder correction.

The JSPB Any diagnostic still returned HTTP 400 / INVALID_ARGUMENT without a
recognized reason, reported by the user. This establishes neither the absence
of server details nor a specific rejection cause. The relay was restored and
its account reported online.

An optional browser-lookup comparison now separates request construction from
the Rust HTTP context. A new explicit popup action forwards one parsed browser
SignInGaia mode-1 body, its matched proof, and service cookies transiently to
the native host. Both extension and Rust validate the exact known four-field
lookup shape, token-free seven-field header, nine-field client-info block, and
type-3 device ID. Mode 0, tokens, other occupied fields, unknown client labels,
and unsupported identifier shapes are rejected. Request bodies are bounded to
2 KiB in the extension; the native frame remains bounded to 32 KiB. This mode
never fetches response bodies from Chrome and returns only the existing source
count or fixed errors. The browser's opaque device ID and request ID are not
stored, exported, or passed into daemon IPC. Existing probe actions continue
to create their own requests. This diagnostic is not a production protocol
client or a pairing implementation. It has not yet been live-tested.

If this comparison succeeds while the independently built request fails, the
body differences become the next investigation target. If both fail, the Rust
HTTP context and captured proof handling remain candidates. The Google browser
request is not replayed unless it passes both read-only validators. If no such
request appears within the existing 120-second window, the comparison times out
without issuing a native request.

The validated browser-body comparison also returned HTTP 400 / INVALID_ARGUMENT,
reported by the user. This makes the independent request-body differences a
less likely explanation, but is not a proof of body acceptance or successful
authentication. The relay was restored and the test tab closed.

The next single-header experiment adds Referer: https://messages.google.com/
only to the browser-body comparison. The Rust transport formerly sent no
Referer. Google's [API-key documentation](https://docs.cloud.google.com/docs/authentication/api-keys)
describes HTTP-referrer restrictions. This motivates the experiment but does
not establish that the Messages key has such restrictions, nor that the exact
browser referrer was observed by our sanitized capture. Only the fixed Messages
origin is sent, with no conversation path or query. Request body, cookies,
authorization, user-agent behavior, and query handling stay unchanged. Other
probe actions remain unchanged. This header change is not yet live-tested.

The origin-only referrer comparison also returned HTTP 400 / INVALID_ARGUMENT, reported by the user. Adding that header did not resolve the rejection. Authentication remains unverified. The relay was restored after the test.

The fixed diagnostics do not identify the rejected argument. A separate opt-in
local inspection now permits the owner to view only the bounded RPC description,
which may include identifiers. It uses the same validated browser lookup and
origin-only referrer. It adds no headers or request fields. The raw description
is excluded from snapshots and exports, retrieved once, and cleared after
60 seconds in the worker and 60 seconds after viewing in the popup. The owner
can review it locally and share a redacted cause. Metadata and full bodies are
never returned. All ordinary probe actions still discard free-form text, and
Debug/Display redact the local-description wrapper. Tests cover opt-in gating,
redaction, one-time retrieval, expiry, and exclusion from snapshots. Stopping
or beginning any capture now invalidates pending native replies unless the
stop is the internal capture-to-native handoff. This local view remains
unverified against the live endpoint.

Local inspection responses allow a bounded 16-KiB native frame because a
2-KiB description can expand when JSON escapes control bytes. Ordinary replies
retain their 1-KiB limit. The description itself remains limited to 2 KiB.

The first local-inspection attempt stopped with cookie_unavailable, reported
by the user. No native request was issued. This can occur when ExtraInfo precedes
the URL event; cookies are intentionally discarded until the exact service
request is matched. The existing reverse-order test covers that fail-closed
behavior. The relay was restored and the test tab closed. A manual retry uses
the same capture rules and inspection request.

The retry reached the live RPC. The owner reported that the local description
identified an unknown `authuser` query parameter. The Rust transport had added
that parameter alongside `X-Goog-AuthUser`. SignInGaia rejects the URL parameter
before processing the lookup. The transport now leaves the endpoint URL unchanged
and retains the account-selection header. A localhost HTTP regression failed
before the fix and passed afterward, checking both the exact request target
and the header. Live authentication with the corrected URL remains pending.

After the URL fix in ed0924f, the owner ran Compare with service cookies and
reported a successful native authentication check with four sources. This action
uses the independently constructed Rust lookup body, fresh request/device IDs,
and no Referer or captured browser body. It confirms the live read-only lookup
with transient matched browser credentials. It does not verify independent
credential acquisition, refresh, pairing, or conversation access. The production
relay was restored and its account reported online; the test tab was closed.
The next implementation checkpoint is source selection and the pairing/session
contract, derived from first-party evidence before any registration request.


## Native initial-send rejection

The live attempts on 2026-10-03 completed source lookup and matched the saved
native registration to the signed-in account. The initial pairing SendMessage
RPC returned HTTP 400. No verification emoji reached Handover, and the owner
reported no new phone request or paired device. The production relay was restored.

The captured first-party `mw_web_only.js` constructs its Gaia messaging client
through `A4a`, `CH`, and `BH` with binary mode disabled. `J_a` selects
`application/json+protobuf`, and the SendMessage descriptor serializes the outer
request as JSON. Its Ditto message field 12 remains a base64 binary payload.
Handover previously sent that outer request as binary protobuf. This is a
transport difference, not proof of the HTTP 400 cause.

The pairing send transport now uses the observed JSON-protobuf format. It
preserves destination field 1, message field 2, authentication field 3, lifetime
field 5 as an int64 string, and recipient identities field 9. The internal
pairing transcript and persisted registration representation are unchanged.
Mock-phone tests cover both initial and confirmation sends, rejection,
cancellation, and acknowledgement failure. Live acceptance remains unverified;
no automatic retry was added.


The first live attempt after switching sends to JSON-protobuf completed account
binding, then failed with `receive_failed` before any emoji prompt. The owner
again reported no new phone request or device. This does not establish send
acceptance: the login loop discarded the receive future's detailed error, and
the receive failure can race the initial send. The relay was restored.

The loop now preserves bounded receive HTTP diagnostics, fixed RPC status
categories, and framing error names. It also emits a separate progress prompt
only after the initial send receives HTTP acceptance. A mock rejection test
reproduced HTTP 401 being collapsed into `receive_failed`; it now retains
`UNAUTHENTICATED` and verifies no initial send occurs. These diagnostics await
live verification. No new phone request was issued during their implementation.


The next live attempt reached account binding and failed with the newly retained
framing category `SessionPreempted`, before initial-send acceptance was reported.
The relay was restored. Inspection found that Handover treated the inactive flag
on every queued RPC response as preemption before checking its kind or request
correlation. Google's `dJ` handling scopes that flag to a current request and
session. This was an overly broad Handover interpretation, not evidence of a
confirmed competing native session.

Pairing projection now ignores unrelated response kinds and carries the inactive
flag privately until the request type, fresh request ID, and selected sender
match the pending pairing phase. Only a matching inactive reply aborts pairing.
Mock tests replay a stale inactive reply before a successful ceremony and verify
that matching preemption causes neither acknowledgement nor confirmation send.
The change awaits another live test; the previous attempt's send outcome remains
uncertain because receive failure can race transport completion.


The following live attempt failed with `Malformed` before initial-send acceptance
was reported. No verification emoji reached Handover, and the relay was restored.
That category did not identify whether stream framing or pairing projection
rejected the data. Offline review found that the projection rejected eventless
records, while the first-party dispatcher treats an absent oneof as a no-op.
An added regression reproduced this rejection; empty and header-only records
now produce no pairing reply and no acknowledgement.

The projection also now distinguishes invalid event layout, binary envelope,
pairing payload, and identifiers using fixed error names. It emits no field
values or message contents. Mock ceremonies include an eventless record and a
stale inactive reply before the valid phone response. These fixes pass offline
checks and await live verification; the last send outcome is still uncertain.


The next attempt failed with `InvalidPairingPayload` before initial-send
acceptance was reported. The relay was restored. Review found that although
preemption had moved after correlation, payload-shape checks still ran before
correlation. A queued stale response could therefore abort the new attempt if it
was streaming, encrypted, empty, or otherwise unsupported for pairing.

Projection now retains bounded correlation metadata and a private validity flag.
Unsupported payload contents are discarded. The dispatcher checks request type,
request ID, and selected sender before acting on preemption or payload validity.
Direct reply-acceptance methods apply the same order. Mock tests reproduce a
stale streaming inactive reply and complete the following valid ceremony. A
matching unsupported payload still aborts with no acknowledgement or
confirmation send. The change awaits live verification.


The next live attempt received HTTP acceptance for the initial pairing send,
then stopped on a correlated inactive-session flag. No emoji was produced.
The production relay was restored. HTTP acceptance is not phone confirmation.

First-party review found that the Gaia pairing backend overrides `Vx()` to
connect with mode `bo:3`. That mode opens receive and resolves readiness without
activating a browser session. Its initial session ID remains empty. The generic
dispatcher treats the inactive flag as preemption only when its active-client
state is already true. The earlier unconditional preemption assumption above
was therefore incorrect for bootstrap pairing.

Native bootstrap now sends an empty session ID for both pairing phases and
validates correlated replies despite the inactive flag. Request type, fresh
request ID, selected sender, payload bounds, and UKEY2 validation remain required
before acknowledgement or confirmation. A mock-phone regression first failed
with the previous behavior and now completes an authenticated ceremony with an
inactive initial reply. Both outgoing envelopes must have empty session IDs.
This fix has not yet been tested against the physical phone.


## Libgm comparison, 2026-10-03

At the user's request, reviewed the installed production dependency
`go.mau.fi/mautrix-gmessages` at `v0.2609.0`, specifically
`pkg/libgm/pair_google.go`, `session_handler.go`, and `longpoll.go`. This was
read-only inspection. No implementation or generated protocol files were copied,
imported, or linked into Handover. These observations identify questions to check
against Google's client, rather than implementation templates.

- Libgm obtains its registration token immediately before pairing using a stable
  browser device identifier and a refresh public key. It updates its browser and
  destination identity from that response. Handover instead loads a previously
  saved registration, checks its identity against a mode-1 source lookup, and
  continues with its saved token. That lookup currently constructs a fresh
  unrelated browser identifier. Google's client persists its registration device
  identifier through `k5a`, passes it into authentication, and supports mode-1
  lookups separately through `$4a`. Token freshness and lookup identity continuity
  therefore need an explicit comparison. This is a discrepancy, not evidence
  that the saved token caused the failure.
- Libgm selects a primary destination registration and starts receive before
  sending client-init. Handover also starts receive first, but uses the newer
  first-party source inventory and byte-valued recipient identities. Libgm's
  string UUID destination must not be substituted without first-party evidence.
- Libgm's response waiter handles initial and final pairing replies by request
  correlation and does not reject them merely because the session is inactive.
  Google's active-state gate independently supports the preceding bootstrap fix.
- Libgm explicitly ignores some unsolicited unencrypted responses while waiting
  for cookie-backed replies. This reinforces the need to correlate before
  validating a pairing payload, which Handover's mock regression already covers.
- Libgm derives the emoji after server-init and only completes pairing after
  client-finish receives a phone response. HTTP acceptance alone is not pairing
  success. Handover retains this distinction.

Next offline work should compare the complete registration-to-pairing identity
chain and both wire envelopes against the first-party client. No further phone
retry or registration mutation was performed for this comparison.


The identity-chain review found a first-party-supported mismatch. `rJ.Qa` gets
the persisted device ID through `k5a` and passes it, with the saved registration
identity, to `a5a` for its mode-1 binding lookup. Handover used the generic
inventory request, which generates a fresh unrelated device ID. Both native
account-binding paths now use the registration's saved device ID. Independent
inventory probes still use fresh IDs, matching Google's separate `l5a` path.

A regression first failed on the mismatched ID. It now verifies the same device
ID before and after persisted-record restoration, fresh per-request IDs, mode 1,
and omission of registration key/token fields. Mock HTTP ceremonies verify the
binding lookup identity, the saved token on receive and both sends, and the
selected destination on both sends.

The refresh-public-key mechanism in libgm should not be transplanted into this
client. The current first-party `rJ.Xr` registers random 32-byte metadata in field
36; Handover already implements that observed path. Google's initialization also
restores saved token payloads and their expiry. Saved-token use therefore is not
by itself a demonstrated bug. No token refresh, re-registration, or remote-state
mutation was performed during this review. The identity fix remains unverified
on the physical phone and does not establish the cause of the missing prompt.


The identity-bound live attempt received initial-send HTTP acceptance but failed
on `InvalidPairingPayload`. The user observed no pairing request or new linked
device. Production relay restoration was verified. No phone confirmation or
verified UKEY2 server-init was obtained.

Review found another unsupported restriction. Google's `B3a.jN` uses outer
reply fields 6 and 7 for generic response-count handling. `H5a` then takes the
first matching initial reply through `Tj(1)`. These fields do not require a Gaia
reply to be non-streaming with a sequence between zero and one, as the native
projection previously assumed. A red regression reproduced rejection of a valid
payload with count metadata; the restriction is removed. Mock ceremonies now
carry that metadata on the initial reply and still require a valid UKEY2
exchange, correlated final confirmation, and validated acknowledgements.

Remaining payload failures have fixed distinct categories for encrypted body,
auxiliary body, missing body, and oversized body. Invalid contents are discarded
before they enter the reply queue. Correlation still precedes these errors.
The malformed protobuf category remains `InvalidPairingPayload`. No field values,
body contents, or secret material are emitted. The live failure's exact condition
was not retained, so this correction is not yet a proven explanation of it.


A targeted follow-up review of libgm confirms that `SessionHandler.receiveResponse`
delivers the first reply matching a waiting request without rejecting fields 6
and 7. `sendGaiaPairingMessage` parses the unencrypted response body afterward.
This supports the response-count correction independently of Google's current
`B3a`/`H5a` flow. Libgm's generic receive handler also attempts session-key
decryption for encrypted and auxiliary bodies before dispatch. That does not
establish that such bodies should be accepted during Handover's unpaired
bootstrap, where confirmed session keys do not yet exist. Those shapes remain
explicit failures rather than silently bypassing validation. Libgm's early
acknowledgement and content logging behavior was not adopted.


The reply-count live test validated the correlated initial phone handshake and
derived the verification emoji. A subsequent request failed with HTTP 400. The
user saw no phone request or new linked device. Production relay restoration
was verified. This establishes server-init and emoji derivation, not completed
pairing. The old progress messages did not identify whether acknowledgement or
final send failed.

The next offline comparison found that native acknowledgement transport still
sent binary protobuf and expected a binary response. Google's `e_a` AckMessages
RPC uses the same JSON-protobuf serializer as its SendMessage RPC, and libgm's
acknowledgement transport uses PBLite. Google's current `lLa` places processed
message IDs in field 2; the older libgm acknowledgement shape is not adopted.
A regression failed on the binary Content-Type. Native acknowledgements now use
the first-party JSON array shape with header field 1 and message IDs field 2,
base64 token encoding, bounded responses, and JSON-protobuf media types.
Intermediate JSON strings are erased after encoding.

The helper now reports initial-acknowledgement HTTP acceptance and final-send
HTTP acceptance separately. This locates a future failure without recording
request contents. Mock ceremonies exercise both JSON acknowledgements and final
confirmation; rejected final acknowledgements still retain confirmed pairing
evidence without retrying the ceremony. The acknowledgement correction has
not yet been verified against Google's live service.
