# Observe RPC structure in normal Chrome

This local research extension uses Chrome's [debugger API](https://developer.chrome.com/docs/extensions/reference/api/debugger) to observe the active Google Messages tab without a remote-debugging browser launch. It does not implement Handover authentication or pairing.

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

## Offline checks

```sh
node --test google-messages/tools/chrome-observer/*.test.mjs
```

Offline checks do not establish that Chrome installation or a live authenticated capture works.
