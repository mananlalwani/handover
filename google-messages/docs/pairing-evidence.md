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
