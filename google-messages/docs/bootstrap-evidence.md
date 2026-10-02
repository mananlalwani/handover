# Bootstrap evidence

The initial observation on 2026-10-02 used anonymous HTTPS requests to
[Google Messages for web](https://messages.google.com/web/). No existing account
session, browser cookies, or adapter credentials were used.

The page referenced a Google-owned bootstrap script under
`www.gstatic.com/_/messagesweb/`. That script was 1,214,288 bytes with SHA-256
`be76064139da1cf62b330b3c0e628d9f989a2d212a43ec317815b35f668e8fd7`.
Its literal RPC paths identified registration, pairing, and messaging services.
This establishes useful observation points, not their request schemas,
authentication rules, encryption construction, or a working protocol client.
A method name alone does not establish permission to call it or a capability.

Pairing observations should come next from Google's own client. Google's
[documented consumer flow](https://support.google.com/messages/answer/7611075?hl=en)
uses Google-account sign-in and phone emoji confirmation. It permits only one
active computer at a time, so activating a new test pairing can interrupt the
current production helper. Keep anonymous exploration separate until an
account/pairing transition is explicitly scheduled.

Protocol field numbers and cryptographic parameters must be established from
first-party observations and validated against actual traffic. Do not fill gaps
from the old adapter or its upstream generated definitions. Raw authenticated
traffic must not be printed, committed, or attached to issues; any future capture
needs private storage and sanitization before it becomes a fixture.

## Script used by normal Chrome

After the user signed in, the normal Chrome page referenced a different public
bootstrap build. Its script URL was read from a DOM `script` element and fetched
anonymously from `www.gstatic.com`, without browser cookies or an account
session. The script was 1,214,247 bytes with SHA-256
`6d2bed34bb2fbc97bf9d4a7e4653a2ad6749bd6e978eb1ccda3533b1027ff4ae`.
The strict indexer found 20 descriptors among 45 candidate path occurrences.
Raw source remains outside Git. Evidence from the initial build must not be
assumed to share source offsets or minified identifiers with this build.
