# Operations: daemon, environment, exit status, troubleshooting

## Daemon lifecycle

`handoverd` owns authoritative runtime state. `handoverctl` and the
Quickshell client are disposable. They connect, read a snapshot, and
rebuild their views. Restarting the daemon rebuilds backend state. A
native peer reconnects without a new pairing ceremony while its trust
record remains and the saved endpoint is reachable.

Install and start the user service:

```
make install-user
systemctl --user status handoverd
handoverctl devices
```

Restart when the client socket or a backend connection is stuck:

```
systemctl --user restart handoverd
```

Remove the user service and binaries:

```
make uninstall-user
```

The unit file `packaging/systemd/handoverd.service` runs one user
service, restarts on failure, and hardens the process with
`NoNewPrivileges`, a private runtime directory with mode 0700, and a
restricted address family set for Unix, IP, and netlink sockets.

## IPC socket and state paths

The daemon listens on a Unix socket derived from the runtime directory:

- Directory: `$XDG_RUNTIME_DIR/handover`, created by the daemon and by
  systemd with mode 0700.
- Socket: `$XDG_RUNTIME_DIR/handover/handoverd.sock`.
- `XDG_RUNTIME_DIR` must be set and absolute. Without it, every command
  that needs the daemon exits 1.

Other state locations:

- Messaging attachment staging and import roots live below
  `${XDG_STATE_HOME:-~/.local/state}/handover`. Operators may select a
  private absolute staging root with `HANDOVER_GMESSAGES_STAGING_DIR`.
  See `docs/gmessages-sidecar.md`.
- Native trust records live in the native backend directory managed by
  `handover-native`.

## Environment variables

- `HANDOVER_GMESSAGES_HELPER`: explicit path to the Google Messages
  helper binary. When unset, `handoverd` resolves
  `handover-gmessages-helper` via `PATH`. When neither exists, the
  messaging subsystem stays dormant and other backends keep working.
- `HANDOVER_GMESSAGES_STAGING_DIR`: private absolute root for helper
  attachment staging. Optional.
- `HANDOVER_CLIPBOARD_MIRROR`: set to `1` to enable Linux-to-phone
  background clipboard mirroring at daemon start. Off by default. The
  `clipboard-mirror` command changes the same flag at runtime.
- `HANDOVER_NATIVE_SMOKE_PORT`: debug builds only. Forces the native
  backend to listen on a fixed port for smoke tests.

## Optional backends

- Native Android is the primary path. KDE Connect is optional
  compatibility for capabilities the native connection lacks.
- Google Messages is optional and runs in a separate helper process.
  Handover receives normalized conversations, messages, statuses,
  capabilities, and opaque identifiers only. Credentials stay with the
  helper. See `docs/gmessages-sidecar.md`.

## Permissions

Android notification access, media control, call control, and
background access are optional permissions. Enable only the
capabilities needed. Locking a phone needs device-admin access.
Placing a real call needs explicit `--confirm` on each invocation.

## Security and privacy

- Native pairing stores the peer certificate fingerprint and requires
  TLS. The Android identity private key stays in Android Keystore.
- Credential bundles travel CLI to daemon to helper over local
  sockets and pipes only. The daemon never persists them. Bundles never
  appear in argv, logs, or crash reports.
- Handover logs identifiers, counts, and state transitions. It does not
  log message bodies, notification text, URLs, tokens, keys, or file
  contents.

## Exit status

`handoverctl` exits 0 on success and 1 on any error. Errors include a
missing or relative `XDG_RUNTIME_DIR`, an unreachable daemon, an
unknown or ambiguous device selector, a rejected request, and invalid
arguments such as a bad `screensaver` action or a `place` call without
`--confirm`. Error text goes to stderr with the prefix `handoverctl:`.

## Troubleshooting

Check the daemon and recent logs:

```
systemctl --user status handoverd
journalctl --user -u handoverd --since today
```

When the socket is missing, confirm the runtime directory exists and
the service is active, then restart the service. When a native peer
fails to reconnect, confirm the trust record remains and the saved
endpoint is reachable. Pair again after a revoke or an app reinstall.
For messaging, run `messages sync ACCOUNT` to ask the helper to
re-emit state, and check the helper binary resolves via
`HANDOVER_GMESSAGES_HELPER` or `PATH`.

## Generated artifacts

Man page and shell completions are generated from the `clap`
definitions, so `--help` output stays the source of truth:

- Man page: `docs/man/handoverctl.1`.
- Daemon page: `docs/man/handoverd.8`, rendered from the daemon
  definition in `handoverctl/src/daemon.rs`. `handoverd` accepts no
  arguments and shuts down gracefully on SIGTERM or SIGINT.
- Completions: `completions/` with Bash, Zsh, Fish, PowerShell, and
  Elvish files.
- Regenerate with `cargo run -p handoverctl --bin handoverctl-gen -- <out-dir>`
  and copy the results over `docs/man` and `completions/`.
  Packaging installs the man page and completions from those
  directories.
- The test suite fails when the checked-in artifacts diverge from the
  `clap` definitions.
