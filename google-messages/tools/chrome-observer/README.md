# Observe RPC structure in normal Chrome

This local research extension uses Chrome's [debugger API](https://developer.chrome.com/docs/extensions/reference/api/debugger) to observe the active Google Messages tab without a remote-debugging browser launch. It also has separate opt-in actions for testing the Rust client's read-only authentication request and registering a native device. It does not pair a phone.

Chrome's debugger permission is powerful. The implementation only attaches to an active HTTPS `messages.google.com` tab. It does not navigate, change network requests, read the browser cookie store, or send messages. A bounded account-read helper for pairing evaluates one fixed expression that returns only the page's `GA_EMAIL` value; the pairing and local-login actions invoke it once. Remove the extension when testing is finished.

## Load locally

1. Open `chrome://extensions` manually.
2. Enable Developer mode.
3. Click **Load unpacked** and select this directory.

These are Chrome's [documented local installation steps](https://developer.chrome.com/docs/extensions/get-started/tutorial/hello-world). This session's browser policy blocks agent access to that page, so the user must perform the installation.

## Capture a read-only test

Pause the existing relay before refreshing the paired Google Messages page. Schedule automatic restoration so a failed test does not leave the relay disabled.

1. Select the signed-in Google Messages tab.
2. Open this extension from Chrome's extensions menu.
3. Choose a bounded duration and click **Start on this tab**.
4. Refresh Google Messages and wait for the conversation list. Do not send anything.
5. Open the extension again, click **Prepare JSON export**, then save the sanitized JSON. Export promptly while the observer is still active.
6. Click **Stop**, close the test page, and restore the existing relay.

Records live only in extension memory. Chrome can discard them when its worker stops. This is an observation tool, not durable account storage.

Saved facts include RPC names, HTTP method/status, allowlisted content types, JSON positions/types, string lengths, and top-level protobuf field numbers/wire types/lengths. They exclude body values, object keys, cookies, account identifiers, tokens, keys, and message contents. Protobuf structure does not establish field meanings; length-delimited payloads are not guessed to be submessages. The structural parser follows the [protobuf wire format](https://protobuf.dev/programming-guides/encoding/).

## Test the native authentication request

Build and install the local [Native Messaging host](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging):

```sh
cargo build -p handover-google-messages --bin handover-google-messages-auth-probe
python3 google-messages/tools/install_native_probe.py \
  --binary "$PWD/target/debug/handover-google-messages-auth-probe"
```

The installer registers only this unpacked extension's ID. If Chrome shows a
different ID, pass its public ID with `--extension-id`. Reload the extension
manually on `chrome://extensions` after updating it. This adds the
`nativeMessaging` permission.

Pause the old relay with automatic restoration before the live test. Select the
signed-in Google Messages tab, click **Test native authentication**, and refresh
the page. Open the popup again to read the result, then close the test tab and
restore the relay.

This mode forwards one observed SignInGaia authorization header, API key, and
optional account selector to the Rust process through Chrome's stdio pipe. It
does not forward cookies or request bodies. These values stay in process memory
and never enter snapshots or JSON exports. Observation and authentication modes
run separately. The native host accepts one bounded frame and exits.

Rust makes its own SignInGaia mode 1 request with fresh identifiers. It accepts
only three observed Google service origins, disables redirects and proxies,
uses established TLS libraries, and bounds the response to 512 KiB. It returns
only a source count or fixed error. An HTTP error is not proof of account
authentication. A source-list result does not establish independent pairing,
session restoration, conversation decoding, or sending.

The separate **Compare with service cookies** button adds the bounded Cookie
header attached to that same matched SignInGaia request. The user approved this
one-time comparison after the cookie-free probe returned HTTP 401. The extension
never reads the profile cookie store. Cookie headers from events that arrive
before their request URL are discarded. If the matched request has no available
cookie header, the comparison stops with `cookie_unavailable` and makes no native
request. There is no automatic retry or switch between the two modes.

Cookies remain in the local native pipe and transient process memory, and Rust
returns them only to the exact observed Google origin. Redirects remain disabled.
They do not enter observer snapshots, exports, logs, daemon IPC, or files. The
cookie-free button still rejects cookies. The first comparison returned HTTP
400; native authentication remains unverified. Error results can include a fixed
RPC status category. They never include Google's error messages or details.

## Offline checks

```sh
node --test google-messages/tools/chrome-observer/*.test.mjs
```

Offline checks do not establish that Chrome installation or a live authenticated capture works.

The popup can also show an allowlisted Google API infrastructure reason from
structured ErrorInfo details. This never includes server messages or metadata.
Unknown or opaque details are discarded. Its availability on the live Google
Messages endpoint remains unverified.

### Register one native device

The separate **Register one native device** action is effectful. It captures
only authorization, API key, account selector, origin, and service cookies from
one matched SignInGaia request. It does not read or forward that browser
request's body. Rust creates and sends a fresh mode-0 registration request,
then stores the unpaired credential under the restricted
`${XDG_STATE_HOME:-~/.local/state}/handover/gmessages-native` directory. The
result does not mean the phone was paired. No automatic retry is made. Pairing
and message sending are separate operations.

### Check pairing readiness

**Check native pairing readiness** reads only Google's `GA_EMAIL` setting from
the active Messages page using a fixed expression, validates it as an email
address, and holds it only until the matching SignInGaia request is forwarded.
The native host then performs one read-only mode-1 source lookup. The email,
cookies, and authorization do not enter snapshots, exports, logs, daemon IPC, or
files. The result reports a source count only; it does not select a phone or
start pairing. This browser-to-native proof has offline coverage but has not
been live-tested.

### Send sign-in proof to Handover

**Send sign-in proof to Handover** captures the same matched request and
transient `GA_EMAIL`, then passes them only inside an opaque, size-bounded
Login bundle through the local daemon to the in-tree helper. The daemon checks
the active helper's Hello name before forwarding this marked bundle, so the
production adapter cannot receive it accidentally. The worker rechecks the
signed-in email when the request is captured and rejects an account change. The native helper
validates the saved account alias and makes a read-only source lookup. It verifies that the registration belongs to the signed-in account and
selects the eligible phone. The daemon reports queue acceptance; the helper's
Pairing event reports readiness or a fixed failure. This action sends no phone
pairing request. Exactly one pending registration must exist for automatic alias
selection.

### Pair a phone with Handover

“Pair phone with Handover” is a separate explicit action. It sends
`gaia_pairing_start` through the same local route and starts one phone-pairing
attempt. Stop the existing relay before a live test, reload this extension,
start the action on the signed-in Messages tab, and refresh. Watch Handover's
Pairing events for the emoji and compare it on the phone.

The helper saves confirmed pairing keys privately before acknowledging the final
reply. It does not save browser cookies or email and does not yet open a usable
messaging session. The popup reports only that Handover queued the operation.
If the attempt is interrupted, inspect the phone before trying again. There is
no automatic retry and no chat message is sent by this operation.

This flow has local HTTP/UKEY2 mock coverage. Native pairing and the updated
extension actions have not been tested against the physical phone.

### Compare the browser lookup body

“Compare browser lookup” is an optional read-only diagnostic. Start it on the
current Messages tab, refresh, and reopen the popup. It passes one validated
mode-1 SignInGaia body and its service cookies through the local Rust host to
the same Google endpoint. This includes the browser's opaque device identifier
and request identifier. Nothing is saved or exported. Both components reject
registration, token-bearing requests, unexpected fields, and unsupported
shapes. If the browser does not issue an accepted lookup, the 120-second window
ends without a native request. The other two probe buttons keep building fresh
independent request bodies. This comparison has not yet been live-tested.

### Inspect a rejection locally

“Inspect rejection locally” is a separate opt-in diagnostic. It makes the same
validated token-free browser lookup as the comparison, including the origin-only
Messages referrer. After a failure, reopen the popup within 60 seconds and click
“View local error description once”. Google's description may include private
identifiers. Review it locally and share only a redacted cause.

This action returns only the RPC description, bounded to 2 KiB, alongside the
fixed diagnostic fields. It never includes error metadata or full response
bodies. Rust Debug and Display remain redacted. The worker keeps the description
out of snapshots and JSON exports, allows one retrieval, and clears it after
60 seconds or on stop/new capture. The popup renders plain text and clears it
60 seconds after viewing or on closing. The description is never persisted.
Other probe actions continue to discard free-form error text. Live availability
of a useful description remains unverified.

Fixed native replies retain their 1-KiB frame limit. The opt-in description reply
has a 16-KiB frame limit to allow JSON escaping of its 2-KiB description.
