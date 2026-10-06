# Authentication access audit

Audited 2026-10-05. No Google credentials were read and no existing account,
session, or phone pairing was changed. The browser experiment used synthetic
credentials on localhost in a new temporary profile.

## Current authentication and retention

The native client uses the following browser proof fields. These are the fields
the implementation validates and sends, not a demonstrated minimum set required
by Google's servers. Dropping individual cookies or headers requires separate
read-only provider tests, followed by checks across the affected operations.

| Field | Current use | Retention |
| --- | --- | --- |
| Authorization | HTTP authentication for registration, source lookup, messaging, and acknowledgements | Desktop Secret Service after account verification |
| Service Cookie header | Cookie-backed HTTP authentication, up to 16 KiB | Desktop Secret Service; the current implementation retains the complete captured header |
| API key | Identifies the Google web API client | Desktop Secret Service with the proof |
| Auth-user index | Optional Google account selector | Desktop Secret Service when present |
| Account email | Provider routing destination in pairing and ordinary native request envelopes | Desktop Secret Service with authentication |
| Endpoint and origin | Three allowlisted API origins and the fixed Messages origin | Stored proof metadata, validated before use |

The account email is not transient after verified login. `credential_store::encode`
serializes the complete `BrowserProof`, including that field. `build_request`
uses the email as the outgoing provider destination. Removing it now would break
restored requests. The public account ID is a random alias, and separate local
registration/pairing records contain no email or browser cookies. Some previous
setup notes incorrectly described the email as transient; those claims have been
corrected. No storage policy or credentials were changed by this audit.

Device IDs, registration tokens, transport keys, and pairing keys are native
client state. Initial browser access need not collect these from Google's web
client. The working flow does not require Google passwords, unrelated browser
cookies, other tabs, chat bodies, response bodies, or exported browser storage.

Local sources:
[Proof validation](../../crates/handover-google-messages/src/lib.rs),
[Credential encoding](../../crates/handover-google-messages/src/credential_store.rs),
[Native routing envelope](../../crates/handover-google-messages/src/session.rs),
[Login account verification](../../crates/handover-google-messages/src/login.rs).

## Current observer permissions

The observer declares `debugger`, `activeTab`, `alarms`, and `nativeMessaging`.
It declares no host permissions. Its code attaches to one explicitly selected
HTTPS Messages tab, bounds capture lifetime and queues, validates the source
endpoint, and forwards selected headers once. It also contains diagnostic body
inspection and export features. These code restrictions do not narrow the
capability granted by `debugger`: that permission supports browser debugging
domains with page execution and network inspection. It is broader than a normal
setup tool needs.
Sources: [Observer manifest](../tools/chrome-observer/manifest.json),
[Observer runtime](../tools/chrome-observer/service-worker.js),
[Chrome debugger API](https://developer.chrome.com/docs/extensions/reference/api/debugger).

The native host allows one installed extension ID. The helper validates
endpoints, proof modes, source ownership, and pairing correlation. These protect
the handoff but do not make the extension or the captured credentials harmless
if compromised. Native Messaging and debugger permissions are separate trust
decisions.
Local sources: [Host installer](../tools/install_native_probe.py),
[Native host](../../crates/handover-google-messages/src/bin/auth_probe.rs).

## Narrower capture candidate

Use a separate setup-only extension with passive `webRequest`, a fixed account
page read through `scripting` and `activeTab`, and Native Messaging. Give it no
`debugger`, `cookies`, blocking-request, or broad browsing permissions. Request
the allowlisted API origins only during explicit setup, then revoke them on
success, cancellation, timeout, and restart recovery. The existing developer
observer should remain a separate tool rather than accumulating production
setup behavior.

Passive `onSendHeaders` can observe requests without modifying or blocking them.
Cookie visibility requires `extraHeaders`. Host access is needed for both the
request destination and initiator. Host grants are origin-wide; the exact RPC,
tab, method, frame, and initiator restrictions must also be enforced by code.
The main page can be read using a fixed `executeScript` function after the user
invokes the extension. Page-derived values remain untrusted input.
Sources: [Chrome webRequest](https://developer.chrome.com/docs/extensions/reference/api/webRequest),
[Chrome permissions](https://developer.chrome.com/docs/extensions/reference/api/permissions),
[Chrome scripting](https://developer.chrome.com/docs/extensions/reference/api/scripting),
[Chrome activeTab](https://developer.chrome.com/docs/extensions/develop/concepts/activeTab).

Chrome's documentation lists Authorization among headers omitted from
`onBeforeSendHeaders`. An empirical distinction matters here: in a synthetic
test, JavaScript-supplied Authorization was visible in both that event and
`onSendHeaders`. The test also observed an HttpOnly Cookie and API key. It used
Helium's Chromium 154.0.8037.92, a local HTTP page, a Manifest V3 extension with
only `webRequest` and localhost host access, and `requestHeaders` plus
`extraHeaders`. Only header-presence booleans were printed. The temporary browser
profile and extension were deleted afterward. This is not a guarantee for
browser-generated HTTP authentication or actual Google Messages traffic.

The next proof is a bounded real Messages header capture and read-only native
account check without debugger. It must reject missing authentication rather
than silently restore broad permissions, and must not register or pair a device
merely to test header access. Only after that passes should it replace the
supported setup tool.

## Remaining trust limits

The current proof is browser authentication, not a documented Messages-only
OAuth grant. Its authority outside the tested Messages operations has not been
measured. Scoping extension hosts restricts what the extension can observe; it
does not reduce the authority of credentials after they reach native code.
Secret Service keeps required authentication out of session-record files and
uses the existing approved desktop storage policy. It does not remove the
requirement to trust Handover and the desktop credential store.

A dependency-free replacement for acquiring this proof has not been found.
The [extension-free investigation](extension-free-setup.md) records the separate
full-browser candidate and its unverified sign-in acceptance. That route also
grants credential access. Removing an extension alone does not eliminate it.

The immediate recommendation is to retire debugger from normal setup through
the passive-capture proof, then measure which cookies and authentication fields
can be removed. Keep the working session and current stored record until the
replacement and its recovery behavior are verified.
