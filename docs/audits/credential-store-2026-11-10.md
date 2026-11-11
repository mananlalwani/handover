# Credential-store validation, November 10, 2026

The real credential-store lifecycle passed with GNOME Keyring 51.1 on the host
and 46.1 in the disposable Ubuntu 24.04 VM. The test used only synthetic browser
proofs and a randomly generated dummy keyring password. It performed no Google
request, account login, phone pairing, or authenticated receive operation.

## Isolation

Run the test with:

```sh
python3 scripts/test-secret-service.py
```

The launcher starts a separate D-Bus with no service activation directories.
It sets a private session-bus address and temporary XDG data, configuration,
state, cache, and runtime directories. It pre-creates `data_home/keyrings` to
prevent GNOME Keyring's legacy home-directory fallback. Inherited keyring control
and display variables are removed. The Rust test verifies those locations before
performing any credential-store operation.

GNOME Keyring runs in the foreground with a private control directory. The
initial dummy password travels through stdin. Later unlocks use GNOME's
[password-unlock extension](https://raw.githubusercontent.com/GNOME/gnome-keyring/master/daemon/dbus/org.gnome.keyring.InternalUnsupportedGuiltRiddenInterface.xml)
on that private bus. Only this test uses a plain Secret Service session to supply
the dummy unlock password. Handover's production adapter continues to use the
secret-service library's encrypted DH sessions.

The launcher stops the test process group, any remaining descendants, and the
private bus on success or failure, then removes temporary storage. The personal
desktop keyring and running Handover daemon were not changed.

## Coverage

`desktop_store_lifecycle` exercises the actual `DesktopCredentialStore` and
`watch_unlocks` APIs against GNOME Keyring:

- An absent provider returns `Unavailable` for writable checks, save, load,
  and delete. The private bus cannot auto-activate another provider.
- An unlocked provider accepts a new proof and an update. Every proof field
  survives loading, and the update leaves exactly one matching Secret Service item.
- A locked collection returns `Locked` for all four operations within a
  two-second test deadline. No unlock prompt is needed or accepted.
- Locking does not wake credential recovery. A real unlock produces a wakeup,
  and the adapter can reload the saved proof.
- Killing and restarting the keyring process preserves the saved item and
  leaves it locked. The existing watcher still receives an unlock notification
  from the new service owner, and loading then returns the updated proof.
- Independently written items with the wrong content type or malformed record
  return `Invalid`. A valid save repairs the single malformed item.
- Duplicate matching items return `Ambiguous` on load and save. Deletion removes
  both, and repeated deletion is harmless. A subsequent load returns no proof.

The integration test replaces the old manually invoked ignored round-trip test.
The replacement remains ignored by ordinary workspace runs because it needs the
launcher. CI installs GNOME Keyring and runs the launcher explicitly.
Existing codec validation, synthetic signal parsing, and helper retry/replay
coverage remain in the workspace suite. No production export, dependency, or
credential-store behavior changed.

## Test development and results

The first harness attempted to unlock a running collection with a second
`gnome-keyring-daemon --unlock` process. The process returned success, but the
collection stayed locked. A direct lock-state assertion established the harness
failure. Using the observed GNOME password-unlock method resolved it. Temporary
diagnostics recorded signal headers only and were removed afterward.

The final lifecycle test passed on both provider versions. Host validation also
passed Rust formatting, strict workspace Clippy, workspace tests, Android debug
build/lint/JVM unit tests, QML lint, 49 JavaScript tests, 20 offscreen sharing
scenarios, launcher checks, and Actionlint for the changed CI workflow.

These results establish GNOME Keyring interoperability and unlock-driven
credential reloads. They do not establish Google authentication, helper network
reconnection, phone activation, graphical unlock prompts, or compatibility with
other Secret Service providers. Existing helper tests cover retry scheduling and
the rule that reconnect never replays mutations; those remain fixture evidence.
The edited hosted workflow was not triggered locally.
