# Optional Google sign-in prototype

This tests whether Google accepts sign-in in a small Qt WebEngine window.
It does not capture authentication, register a device, pair a phone, or change
Handover's running Messages session. Successful sign-in is not yet a completed
Handover setup flow.

Build and run:

```sh
make google-login-probe
target/google-login-probe/handover-google-login-probe
```

Requires CMake, a C++17 compiler, and Qt 6.8 or later with Widgets and
WebEngineWidgets. Normal Handover builds, installation, and runtime do not
require these libraries. The prototype is deliberately excluded from release
archives and installers until real sign-in and the native handoff are verified.

The window opens Google's account chooser with the Messages configuration page
as its destination. Sign in, then close the window after the configuration page
appears. If Google refuses sign-in, close it and report the displayed error.
Do not navigate to the Messages conversation list for this test.

An unnamed off-the-record profile keeps website cookies and cache in memory.
The window has no access to existing browser profiles, cancels downloads, denies
site permissions, rejects new windows and non-Google top-level navigation, and
closes after ten minutes. It uses Qt's normal certificate validation, user agent,
and browser sandbox. No web-to-native bridge, request observer, account parser,
or credential export exists in this prototype. JavaScript console messages are
discarded. The origin indicator omits URL paths and query strings.

The local MinSizeRel executable is 36,520 bytes with GCC 16 and Qt 6.11.2.
It links to shared libraries; this number excludes the browser engine.
Arch's Qt WebEngine package is about 282 MiB installed. Existing installations
share that engine, while a new installation must supply it and missing runtime
dependencies. An optional installer component is the intended final packaging,
subject to successful authentication testing.

The smoke check loads only `about:blank`, verifies the private-profile settings,
and returns a failure if loading fails or exceeds fifteen seconds:

```sh
xvfb-run -a target/google-login-probe/handover-google-login-probe --self-test
```

References: [Qt private profiles](https://doc.qt.io/qt-6/qwebengineprofile.html),
[Arch Qt WebEngine package](https://archlinux.org/packages/extra/x86_64/qt6-webengine/),
[Google embedded OAuth policy](https://developers.google.com/identity/protocols/oauth2/policies#use_secure_browsers).
The policy is a compatibility concern. It does not establish whether this
non-OAuth Messages sign-in succeeds; that requires a real account test.
