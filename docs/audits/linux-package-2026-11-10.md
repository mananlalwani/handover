# Linux package validation, November 10, 2026

The local 0.4.1 x86_64 Linux tarball passed isolated installation and upgrade
checks. No installed binaries or user service files were replaced. The running
user daemon retained PID 2421909 and its original start timestamp.

## Fixes

Generated systemd service commands now use the chosen installation prefix.
The shared renderer escapes systemd specifiers, quotes, backslashes, and dollar
signs. It uses `/usr/bin/env` to execute the literal path without a shell.
Source and tarball installation use the same renderer.

Installation now requires a successful `handoverctl devices` request. An old
socket file no longer counts as readiness. Each request has a one-second timeout,
and the installer bounds its retries.

`BUILD.txt` records the source commit, working-tree state, Rust compiler, and
builder glibc. CI runs the installer fixtures, and the release workflow validates
its built archive before publishing it.

## Installer checks

`scripts/test-install-from-dist.py` invokes the real installer with private
prefixes and XDG directories. Its systemctl fixture records expected service
operations and never forwards them to the installed daemon. Its private IPC
fixture answers readiness requests from the installed CLI.

Passed cases include clean installation with optional setup disabled, setup
enabled, overwrite upgrades, and an upgrade from the existing local 0.4.0
tarball to 0.4.1. The checks compare installed binaries and runtime assets to
the archive, verify ELF dependencies and CLI version, preserve synthetic state,
and exercise the launcher with a Quickshell fixture. They reject stale sockets
and unresponsive IPC servers within a bounded time.

The custom-prefix case includes spaces, Unicode, percent signs, dollar signs,
and quotes. `systemd-analyze` verifies rendered units. A disposable transient
user service executes the rendered command against a marker executable.
This command test uses a fixture executable, not the release daemon.

These checks establish installer behavior. They do not establish compatibility
with real saved account state or prove that systemctl started a daemon.

## Real runtime check

The unpacked release daemon ran with fresh private XDG directories inside an
unprivileged user/network namespace. The real packaged CLI completed a devices
request, the IPC socket had mode 0600, and the native listener accepted a TCP
connection on port 24837. The test stopped its daemon afterward.

The existing systemd smoke script cannot isolate a release binary's native
port, because its alternate-port override is debug-only. Its release invocation
reached IPC but encountered the installed daemon's occupied port. The private
network namespace supplied the separate release startup check. Multicast
discovery was not verified there because the namespace had only loopback and
the kernel did not provide the attempted dummy interface.

## Other checks and limits

Rust formatting, strict workspace Clippy, and workspace tests passed. Android
debug build, lint, and JVM unit tests passed. QML lint, 49 JavaScript tests,
20 offscreen sharing scenarios, launcher checks, and desktop-file validation
passed. Shell syntax checks passed; ShellCheck was unavailable.

This artifact was built on the local Linux host, not the hosted Ubuntu 24.04
release builder. Its BUILD.txt records its actual glibc requirement. Ubuntu
compatibility, physical-phone workflows, account authentication, and live
desktop interactions were not validated in this packaging pass.

The subsequent [Ubuntu 24.04 validation](linux-ubuntu-24.04-2026-11-10.md) built
and tested a separate Ubuntu artifact. It also found and fixed a real user-service
startup failure under Ubuntu's default AppArmor policy.
