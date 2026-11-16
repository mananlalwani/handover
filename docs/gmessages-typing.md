# Incoming typing

The native helper receives direct RCS composing signals through a separate
Tachyon connection and publishes normalized `TypingState` updates. The ordinary
phone-relay receive stream does not carry these signals.

## First-party evidence

Google's public `mw_rcs_chat` and `mw_conversation` modules from build
`messagesweb.mw.en.HdTbpeC64eA.O` use [RFC 3994](https://www.rfc-editor.org/rfc/rfc3994)
`isComposing` XML with active/idle states and a 60-second refresh. The observed
RCS module SHA-256 was
`19d3834da42622b98faf6fa6ea959f66f334363524aec2136073daa96858e88c`.
Registration uses `Registration/SignInSecondary` and a separate receive stream.
No third-party implementation or generated definitions were used.

## Receiver behavior

Authenticated phone settings supply one RCS identity and service endpoint.
Registration uses the saved account proof, persisted web device identity, and a
fresh P-256 public key. Tokens and wire models remain inside the native client.
Transient failures use bounded backoff; authentication and registration rejections
wait for new settings or a new phone-relay receive owner.

Plaintext composing events require an exact participant-address match to one
known direct RCS conversation. Unknown, ambiguous, group, and encrypted events
remain unsupported and unacknowledged. Supported events are acknowledged after
publication. Active state expires using the refresh interval or RFC default of
120 seconds. Duplicate, stale, future, and reordered signals cannot revive it.
Idle events, changed settings, and disconnection clear it. XML size and node
counts are bounded; DTD processing is disabled.

## Registration status

An earlier live check reached registration and held a receive connection for
55 seconds. The November 11 check instead failed with RPC status 3,
`InvalidArgument`. A connected messaging account does not establish that this
separate typing receiver works. Real incoming typing still needs verification.

Google's public module from build `comms-messages.web-server_20261110.01_p0`,
module version `messagesweb.mw.en_US.nzGU8ZMoY3I.O`, had SHA-256
`537120b73848685202260f13d960b8c3491656538daa640b3da020abbbd7f28c`.
Its declared registration layout still matched the native request's account,
phone, web-device information, SPKI key, and receive features. The inspection did
not establish the rejected argument. Updating the observed client version did
not resolve it, and that change was reverted. Probes stopped after one
`ResourceExhausted` response.

The next step is comparing a successful registration from an already-paired web
session. The attempted browser comparison had no authenticated Messages session.
Keep request bodies, keys, addresses, tokens, and authenticated captures out of
logs and committed records.
