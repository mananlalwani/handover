# Google Messages observer

This contributor tool observes bounded RPC metadata on the active Google Messages
tab. It also provides explicit native authentication and pairing diagnostics.
Normal account setup uses the installed browser flow in the
[user guide](../../../docs/user-guide.md#browser-setup); it needs no extension.

Load this directory through Chromium's **Load unpacked** action only when debugging
the provider. Register its Native Messaging host with `install_native_probe.py`
in the parent directory. Never run diagnostic pairing alongside an active
Handover or Google Messages web session.

Observations remain in extension memory until explicitly exported. Exports omit
cookies, credentials, message contents, and payload values. Authentication actions
pass a matched request's proof directly to the local native host. Treat any
credential bundle as secret and keep it out of arguments, logs, and Git.

Run the observer tests with:

```sh
node --test google-messages/tools/chrome-observer/*.test.mjs
```
