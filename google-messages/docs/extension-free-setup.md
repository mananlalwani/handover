# Extension-free Google Messages setup

Updated 2026-10-05. The optional setup component is implemented and locally
installed. The user confirmed fresh phone pairing, daemon restart recovery,
and Disconnect followed by connecting again in the installed UI. The pairing
symbol stays visible through subsequent handshake updates. These are live
results for this account and desktop; other discovered browsers remain unverified.

## Current setup flow

Install with `make install-user GOOGLE_MESSAGES_SETUP=1`, or use the release
installer's `--with-google-messages-setup` option. The component needs Python 3
and an installed Chromium browser. It does not download a browser or require
an extension.

In Messages, click **Connect Google Messages** and choose a browser. Sign in in
the normal window, then choose **Exit** in its menu. Handover reopens its own
temporary profile with a private DevTools pipe, captures one allowlisted
Messages authentication request, then closes the browser and removes the
profile before native setup. The normal browser profile is never accessed.

A unique saved registration and phone match refreshes that account's credentials.
Otherwise native setup registers a device and shows the confirmation symbol.
Confirm that symbol on the phone. Required Google authentication is saved in
desktop Secret Service. Handover reports connected only when the daemon reports
an authenticated, connected account, then offers **Done**.

The daemon pauses only the messaging helper under a fifteen-minute setup lease.
Other continuity features keep running. Cancellation releases the lease, and
expiry recovers from a crashed setup process. Interrupted registration or
pairing is not retried automatically. Check linked devices on the phone before
repeating a failed attempt.

**Disconnect** beside the account selector removes that account's local saved
credentials after confirmation. It does not revoke the linked device on the
phone. Remove that device separately in Google Messages if desired.

## Verification

- Official Chrome normal sign-in accepted the user's phone passkey. The bounded
  read-only native authentication handoff passed during a receiver pause.
- The live setup lease stopped the helper while the daemon stayed active, then
  restored the previously online account.
- The installed full setup reached connected for an existing pairing. The user
  subsequently confirmed fresh pairing, restart recovery, and disconnect/reconnect.
- Native and Python regression tests cover separate verification-symbol progress
  and forwarding. The user confirmed the emoji display fix in Handover.
- Browser discovery, capture bounds, cancellation, lease expiry, and rejected
  setup modes have automated coverage. The Rust workspace, Python setup tests,
  and QML checks pass.

Discovery lists installed browsers; it does not establish Google sign-in support
for every listed browser. The exact browser used in the final full-flow checks
was not recorded. Official Chrome's sign-in and read-only handoff were verified
separately.

## Investigation findings

The Qt WebEngine prototype stalled on the required phone/Bitwarden passkey and
is not packaged. Both Helium and official Chrome rejected the original launch
with debugging enabled before sign-in. Normal Chrome sign-in succeeded; reopening
that temporary profile with a private pipe preserved the session and allowed
the native handoff. No sign-in bypass or user-agent spoofing was added.

The capture matches only POST `SignInGaia` requests on allowlisted HTTPS hosts.
It forwards bounded authentication headers over private stdin and never exports
request bodies, response bodies, or a network trace. Unmatched headers are not
retained. Early extra-header events are discarded and can make capture fail
closed. Account selection uses a narrowly scoped page read during full setup.
The `--local-read-only` probe rejects registration and pairing modes.

The development probes remain available for isolated checks:

```sh
python google-messages/tools/chromium_login_probe.py --self-test
# Only during a scheduled receiver pause:
python -B google-messages/tools/chromium_login_probe.py \
  --read-only-auth-probe target/debug/handover-google-messages-auth-probe
```

## Evidence

Google's current setup guide requires the same Google account on phone and web,
followed by an emoji confirmation on the phone. It says QR pairing is no longer
available in the US. QR registration RPCs in older captures therefore do not
establish a usable replacement for this user's account setup.
Source: [Google Messages setup](https://support.google.com/messages/answer/7611075?hl=en).

Google documents desktop OAuth with a system-browser redirect for supported
Google APIs. Its published scope registry does not list a consumer Google
Messages companion scope. Google Chat scopes concern a different product.
This is a finding about the reviewed public documentation, not proof that no
private authorization route exists.
Sources: [Desktop OAuth](https://developers.google.com/identity/protocols/oauth2/native-app),
[Published Google scopes](https://developers.google.com/identity/protocols/oauth2/scopes).

Handover's first-party observations and working native client instead use
cookie-backed `SignInGaia` authentication. The existing extension obtains the
required request headers through DevTools events, not an OAuth callback.
The initial cookie-free native request failed with HTTP 401. That observation
does not prove every alternative authentication mechanism would fail.
Local sources: [Pairing evidence](pairing-evidence.md#original-read-only-authentication-probe),
[Capture implementation](../tools/chrome-observer/service-worker.js),
[Native proof model](../../crates/handover-google-messages/src/lib.rs).

Chrome 136 and later ignore remote-debugging port and pipe switches for the
default data directory. A separate non-default profile is required. This
precludes promising to attach automatically to the normal signed-in profile
through those switches.
Source: [Chrome debugging changes](https://developer.chrome.com/blog/remote-debugging-port).

Ordinary page scripts cannot read HttpOnly cookies, and the browser restricts
cross-origin document access. A localhost setup page or a custom URL callback
does not inherit access to Google's authenticated browser session. A bookmarklet
is not an established replacement for the network-level capture used here.
Sources: [RFC 6265, HttpOnly](https://datatracker.ietf.org/doc/html/rfc6265#section-4.1.2.6),
[HTML cross-origin access](https://html.spec.whatwg.org/multipage/nav-history-apis.html#security-infrastructure-for-window-windowproxy-and-location-objects).

An embedded QtWebEngine or similar login window needs a compatibility test. Google
restricts OAuth in controlled embedded user-agents and documents sign-in
rejection in those environments. A full-browser process is a different
implementation, but using DevTools does not guarantee that Google accepts its
sign-in. The earlier isolated Helium sign-in was rejected; its exact cause was
not isolated. No anti-detection changes or sign-in bypass are proposed.
Sources: [Google browser policy](https://developers.google.com/identity/protocols/oauth2/policies#use_secure_browsers),
[Google user-agent errors](https://developers.google.com/identity/protocols/oauth2/native-app#disallowed_useragent),
[Earlier local observation](pairing-evidence.md#observed-pre-authentication-pairing-traffic).
