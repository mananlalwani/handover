# Observe RPC structure in normal Chrome

This local research extension uses Chrome's [debugger API](https://developer.chrome.com/docs/extensions/reference/api/debugger) to observe the active Google Messages tab without a remote-debugging browser launch. It also has a separate opt-in mode for testing the original Rust client's read-only authentication request. It does not pair a phone.

Chrome's debugger permission is powerful. The implementation only attaches to an active HTTPS `messages.google.com` tab and uses network observation commands. It does not navigate, inject scripts, read cookies, change requests, or send messages. Remove the extension when testing is finished.

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
