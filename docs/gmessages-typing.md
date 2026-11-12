# Incoming typing investigation

Handover's native helper receives direct RCS composing signals through a
separate authenticated Tachyon connection and publishes normalized `TypingState`
updates to the daemon and Messages page.

During a user-assisted observation on October 8, 2026, the phone showed typing
while the native observer received 12 authenticated pushes: activation,
settings, a user alert, message changes, and conversation changes. It received no
recognizable typing update. The user also confirmed the indicator appeared in
Google Messages for web.

Inspection of the first-party scripts loaded by that web conversation identified
a separate RCS client. Its typing serializer uses the IETF `isComposing` XML
format, with active and idle states and a 60-second refresh value. The RCS client
uses its own Tachyon registration and receive connection, including
`Registration/SignInSecondary`. Handover's existing phone-relay receive stream
is not that connection.

Sources were Google's public `mw_rcs_chat` and `mw_conversation` script modules
loaded by the shared Messages tab, version `messagesweb.mw.en.HdTbpeC64eA.O`.
The observed RCS module bundle SHA-256 is `19d3834da42622b98faf6fa6ea959f66f334363524aec2136073daa96858e88c`.
No third-party implementation or generated protocol definitions were used.
The standard is [RFC 3994](https://www.rfc-editor.org/rfc/rfc3994).

The helper extracts one RCS identity and service endpoint from authenticated
phone settings. It registers through `SignInSecondary` using the saved Google
account proof, its persisted web device identity, and a fresh P-256 public key.
Reconnects reuse the device identity. Tokens and protocol models stay inside
the native client. The RCS connection ends with its phone-relay receive owner.
Transient failures retry with a bounded backoff; authentication and registration
rejections wait for new settings or a new receive owner.

Plaintext composing events bind only to one known direct RCS conversation through
an exact participant address match. Unknown senders, ambiguous matches, group
events, and other content types remain unacknowledged. A supported event is
acknowledged only after the helper publishes it. Active signals expire according
to their refresh interval, with the RFC's 120-second default. Duplicate, old,
future, and reordered signals cannot revive an indicator. Idle signals, changed
registration settings, and account disconnection clear it. XML input and parser
node counts are bounded, and DTD processing is disabled.

Multiple distinct RCS SIM identities and encrypted composing content remain
unsupported. The existing phone-relay stream continues to own messages, history,
and sends. The separate RCS receiver does not acknowledge unrelated messages or
clear the inbox.

A live check confirmed registration and an open receive connection for 55 seconds.
Unit and transport tests cover composing parsing, sender binding, stale events,
publication before acknowledgement, expiry, and daemon disconnection. A real
incoming typing event still needs user-assisted verification.

## November 11 registration check

The native Messages UI displays and clears synthetic typing events in its GUI
test, but the live RCS registration currently fails with RPC status 3,
`InvalidArgument`. The service's error text was generic. Fixed helper lifecycle
diagnostics distinguish this failure from waiting for settings or an open receive
connection. The ordinary messaging account recovered as connected and
authenticated; that does not establish a working RCS typing receiver.

Inspected Google's publicly served `mw_rcs_chat` module from build
`comms-messages.web-server_20261110.01_p0`, module version
`messagesweb.mw.en_US.nzGU8ZMoY3I.O`. Its SHA-256 is
`537120b73848685202260f13d960b8c3491656538daa640b3da020abbbd7f28c`.
The declared SignInSecondary layout still uses the linked account identity,
phone identity, web device information, SPKI public key, and receive features
represented by the native request. This inspection did not establish which
argument the service rejected.

Temporary debug instrumentation checked only request metadata and fixed error
categories. The native identity had E.164 shape, its device had web-device shape,
and its SPKI public key had 91 bytes. No addresses, keys, tokens, or request bodies
were printed or retained. Testing the current observed client version on the RCS
path did not resolve the failure, so that change was reverted. One attempt
returned RPC status 8, `ResourceExhausted`; further manual registration probes
were stopped. The installed release retains the tested request behavior.

The next comparison needs a successful first-party registration from the user's
already-paired web session. The shared browser reached Google's sign-in page
and had no authenticated Messages session. No new Google login or phone pairing
was attempted. Incoming typing remains unresolved.
