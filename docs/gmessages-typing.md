# Incoming typing investigation

Handover's daemon and Messages page already accept normalized `TypingState`
updates. The native Google Messages helper does not currently produce them.

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

Implementation must authenticate and receive the separate RCS events, bind the
sender and conversation to known normalized identities, and publish active and
idle states through the existing helper contract. Credentials and protocol types
must remain inside the native helper. Expiry must follow the composing signal's
refresh semantics. Message updates, delivery receipts, and ordinary message
activity must not be presented as typing evidence.

Incoming typing remains unimplemented. The observer's bounded startup queue was
raised to the same 16-entry limit as the live receiver so this investigation
could run without failing on the startup inbox burst.
