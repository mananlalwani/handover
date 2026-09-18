# Native backend API research

Research date: 2026-09-16. The sources below are first party Android, Java,
Rustls, and IETF documentation. They guide the first native link, which carries
presence and battery state only.

## Android lifecycle and local discovery

Android's [`NsdManager` API](https://developer.android.com/reference/android/net/nsd/NsdManager)
implements DNS Service Discovery over multicast DNS. Registration and discovery
are asynchronous, and service records provide the instance, host, port, and
optional TXT attributes. The Android guide also documents that service names
may be changed for conflict resolution
([NSD guide](https://developer.android.com/develop/connectivity/wifi/use-nsd)).
Handover therefore treats the advertised name, address, port, and TXT version as
untrusted connection hints; the persistent certificate fingerprint remains the
peer identity.

On Android 12 and later, background foreground-service starts are restricted.
The documented exceptions include a user-visible transition and
`CompanionDeviceManager` associations
([background start restrictions](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start)).
`CompanionDeviceManager` can also report companion presence and request the
background execution and data permissions when they are actually needed
([API reference](https://developer.android.com/reference/android/companion/CompanionDeviceManager)).
The companion app starts its connected-device foreground service from an explicit
user action and uses the service for the active LAN session. Reconnection must
handle process and network loss; it cannot assume that `START_STICKY` or an NSD
advertisement means a trusted connection exists.

Android 14 requires each foreground service to declare an appropriate service
type and permission
([foreground-service changes](https://developer.android.com/develop/background-work/services/fgs/changes)).
The current service uses `connectedDevice`; notification permission and any
future companion-device background permissions remain explicit product choices.

## Identity and transport choice

The Android Keystore keeps private key material non-exportable and can enforce
key use restrictions outside the app process
([Keystore system](https://developer.android.com/privacy-and-security/keystore)).
Hardware backing is device-dependent, so the protocol must not claim hardware
attestation unless it verifies the attestation chain and security level as
described in Android's [key-attestation guidance](https://developer.android.com/privacy-and-security/security-key-attestation).

For the Rust and Kotlin boundary, standard TLS is the smallest interoperable
choice. Java's [`SSLSocket`](https://docs.oracle.com/en/java/javase/26/docs/api/java.base/javax/net/ssl/SSLSocket.html)
provides confidentiality, integrity, and peer authentication, and supports
requesting client authentication. Rustls documents TLS 1.2 and 1.3 support and
explicit server/client certificate configuration
([rustls overview](https://rustls.dev/docs/rustls/index.html),
[certificate configuration](https://rustls.dev/docs/rustls/struct.ConfigBuilder.html)).
The current daemon uses OpenSSL's maintained TLS implementation with TLS 1.3
and peer certificates; the Android side should use the platform TLS provider.
Pairing pins the peer's certificate fingerprint after both users approve the
comparison code. Discovery never supplies trust, and a certificate whose
fingerprint is absent or revoked is rejected before state is accepted.

This avoids a custom key exchange, cipher, or encrypted record format. TLS
record framing remains the cryptographic boundary; Handover's application
messages add only a bounded, versioned length prefix and JSON schema validation.

## DNS-SD constraints

RFC 6763 defines DNS-SD service instances using PTR discovery plus SRV and TXT
records. SRV supplies the target host and port, while TXT carries structured
key/value attributes ([RFC 6763](https://datatracker.ietf.org/doc/html/rfc6763)).
The native advertisement uses `_handover._tcp.local.` and a protocol-version
TXT value. TXT values are metadata, not credentials, and the implementation
must tolerate stale, duplicated, renamed, or malicious advertisements.

## Scope boundary

These APIs support persistent identities, explicit pairing, authenticated TLS,
LAN discovery/reconnection, peer revocation, presence, and battery reporting.
The notification-listener service (`NotificationListenerService`) supplies the
platform notification stream for the native notification path: posted/removed
callbacks, `activeNotifications` snapshots on connect, `cancelNotification`
for dismissal, action `PendingIntent` invocation, and `RemoteInput` replies.
The media session manager (`MediaSessionManager`) with active-session
callbacks supplies the native media path through the same listener grant:
per-controller playback state, metadata, and transport controls, with
`seekTo` as the only genuine seek primitive.
They do not justify adding clipboard, sharing, SMS,
calls, camera, audio, or display messages to the native protocol yet.
