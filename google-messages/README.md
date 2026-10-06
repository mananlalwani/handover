# Handover Google Messages client

Handover's independent Google Messages client supports SMS and RCS through
Google Messages on the phone and Google's companion service. The native helper
is bundled and selected by default.

The [Rust client](../crates/handover-google-messages) handles authentication,
registration, UKEY2 pairing, encrypted updates, paged history, text and media,
replies, reactions, delivery evidence, and phone unpairing. Google protocol types
and credentials stay inside the helper. The daemon owns the normalized account,
conversation, message, and outgoing-operation state exposed to clients.

Sending requires an authenticated phone capability response. Acceptance does not
prove delivery. Interrupted sends retain an unknown outcome and are never retried
automatically. Disconnect requires a confirmed phone unpair result before removing
a confirmed account's credentials.

## Setup

Install with `make install-user GOOGLE_MESSAGES_SETUP=1` or the release installer's
`--with-google-messages-setup` option. Setup uses Python 3 and an installed Chromium
browser; it needs no extension or bundled browser. See the
[browser setup guide](../docs/user-guide.md#browser-setup).

Confirmed pairing records use restricted local files. Required Google
authentication is saved in desktop Secret Service. The temporary browser profile
is deleted before native pairing. Authentication and keys never enter normalized
Handover state.

## Integration and licensing

The helper implements [helper IPC v1](../docs/gmessages-helper.md). Other backends
and public clients use the same Handover models. See the
[known limitations](../docs/KNOWN_LIMITATIONS.md) for unavailable operations.

Handover's client code is covered by the root MIT license. The included
[UKEY2 dependency](../third_party/ukey2/README.md) retains Apache-2.0 licensing.
No separate checkout is required.
