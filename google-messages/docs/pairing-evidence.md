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
