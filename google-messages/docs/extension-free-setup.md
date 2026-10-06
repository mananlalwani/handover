# Extension-free Google Messages setup

Investigated 2026-10-05. The current Handover session was left running.
No account credentials, Google registration, phone pairing, or messages were
accessed during this investigation.

## Conclusion

The strongest candidate is a setup window using full Chrome with a temporary,
Handover-owned profile and a private DevTools pipe. Handover could capture the
same narrowly scoped authentication proof as the existing extension, then use
its existing native registration and pairing flow. The local browser connection
works; Google sign-in acceptance and the authenticated handoff remain untested.

Opening the user's existing browser alone does not provide that handoff.
No supported consumer Messages OAuth callback was found. An ordinary Google
sign-in callback must not be treated as authorization for Messages.

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

An embedded QtWebEngine or similar login window is a poor first choice. Google
restricts OAuth in controlled embedded user-agents and documents sign-in
rejection in those environments. A full-browser process is a different
implementation, but using DevTools does not guarantee that Google accepts its
sign-in. The earlier isolated Helium sign-in was rejected; its exact cause was
not isolated. No anti-detection changes or sign-in bypass are proposed.
Sources: [Google browser policy](https://developers.google.com/identity/protocols/oauth2/policies#use_secure_browsers),
[Google user-agent errors](https://developers.google.com/identity/protocols/oauth2/native-app#disallowed_useragent),
[Earlier local observation](pairing-evidence.md#observed-pre-authentication-pairing-traffic).

## Local feasibility check

Installed Chrome 154.0.8037.97 started successfully with a new private temporary
profile, `--remote-debugging-pipe`, and `about:blank`. DevTools returned the
browser version and one blank page. No extension, debugging TCP listener,
existing browser profile, or signed-in Google page was used. Chrome was closed
and the temporary profile removed. This verifies process and pipe access only.

## Proposed setup experience and next proof

The user clicks Connect Google Messages. Handover opens a full Chrome setup
window, the user signs in, and Handover captures one matched Messages request.
The existing native client then registers and pairs its own device. The user
confirms on the phone. Afterward Handover closes the setup window and removes
the temporary profile; the required authentication stays in the already
approved desktop credential store. Reconnects and renewal use that store.

This trades extension installation for signing in once in a separate window.
It does not reuse the normal browser's existing login. Chrome must be available
unless a later packaging decision supplies a supported full browser.

Before changing the supported setup flow, build a bounded login-only prototype
and test real sign-in plus one read-only native account check. It should capture
only the validated Messages request and transient account selection, retain no
page contents or network trace, and keep secrets out of arguments and logs.
Browser control must end on success, cancellation, or timeout. The authenticated
test needs a scheduled pause of Handover's receiver because Google allows only
one active computer session. It must restore the existing session afterward.
No fresh registration or phone pairing is needed for that first proof.

If Google rejects the login, retain the working extension flow and record the
failure. The extension-free route is not a completed setup feature yet.
