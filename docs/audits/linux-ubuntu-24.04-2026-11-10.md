# Ubuntu 24.04 package validation, November 10, 2026

## Environment

Validation used an isolated QEMU/KVM VM with two CPUs, 3 GiB RAM, and a private
NAT network. Only SSH was forwarded, on host loopback. No host directories,
credentials, device sessions, or systemd services were shared with the guest.

The guest reported Ubuntu 24.04.5 LTS, kernel 6.8.0-147-generic, glibc 2.39,
OpenSSL 3.0.13, and Rust 1.99.1. Its base was the official
[Noble cloud image](https://cloud-images.ubuntu.com/noble/current/), build
20261017. Its SHA256 matched the official HTTPS checksum manifest:

```text
04e8f2461a16a44e78a370284471f29616cdd9baf1893df25807b71f985b6c94
```

The VM used the documented
[cloud-init NoCloud setup](https://docs.cloud-init.io/en/latest/howto/launch_qemu.html).
The project source came from a local Git bundle. No remote was added or changed.

## Reproduced portability failure

The earlier Arch-built tarball failed its ELF dependency check inside Ubuntu.
It required `libssl.so.4` and `libcrypto.so.4`, which Ubuntu 24.04 does not supply.
The test rejected it before running the installer. The temporary test account
was removed after this failure.

The Ubuntu source build initially failed because CMake was missing. Installing
`cmake` allowed compilation to continue. CI, release setup, and contributor
instructions now name that dependency explicitly.

`BUILD.txt` now records the builder's OpenSSL version alongside glibc, Rust, and
source provenance. The README distinguishes Ubuntu-built release libraries from
the requirements inherited by a local build.

## Automated coverage

The existing archive fixture checks remain responsible for package layout,
optional assets, launchers, stale sockets, and bounded IPC readiness.

`scripts/test-package-runtime.py` adds the installed-service boundary. It needs a
disposable systemd environment and root access to create temporary users.
Each case gets its own home, state, runtime directory, and user manager. The test
runs the actual installer without a systemctl proxy, checks the loaded service
file and executable against the archive, and verifies real CLI access, socket
permissions, and native TCP listener ownership. Cleanup attempts every account
and manager removal even if an earlier cleanup step fails.

The clean case uses a prefix containing spaces, percent signs, dollar signs, and
quotes. Reinstallation checks a new service invocation, unchanged native
certificate, preserved synthetic state, and replaced QML assets. The separate
upgrade case installs the previous package first. If its service starts, the
same checks apply. A previous service that fails with `218/CAPABILITIES` remains
a failed baseline, and the upgrade must recover it. This does not validate an
authenticated Google Messages session or phone state migration.

The fixed upgrade baseline is the
[published 0.4.0 Linux package](https://github.com/mananlalwani/handover/releases/download/v0.4.0/handover-0.4.0-x86_64-unknown-linux-gnu.tar.gz),
with SHA256:

```text
d0417e43b3d106e21595879cc399486fb0c97acfaf95f7691bbb82fcfb99aa50
```

Both the Ubuntu 24.04 CI package job and release job verify that checksum before
using the baseline. Release publication follows the archive fixtures and real
service checks.

## Service startup defect and fix

The first real installed-service check failed before launching the daemon:

```text
handoverd.service: Failed to drop capabilities: Operation not permitted
handoverd.service: Main process exited, code=exited, status=218/CAPABILITIES
```

The failure reduced to this command in the guest's ordinary systemd user manager:

```sh
systemd-run --user --wait --collect -p CapabilityBoundingSet= /usr/bin/true
```

It exited with status 218. The command without that property passed.
`ProtectKernelModules=yes` independently reproduced status 218. Every other
hardening property in the unit passed the same individual probe.
The kernel audit log recorded AppArmor denials for `setpcap` and `sys_admin`
inside `unprivileged_userns`. This agrees with Ubuntu's documented default
[restriction on capabilities in unprivileged user namespaces](https://documentation.ubuntu.com/release-notes/24.04/).

Both daemon and desktop user-unit templates now omit those two incompatible
settings. The source-tree smoke script uses the same policy. Other settings,
including `NoNewPrivileges`, remain. No AppArmor profile or kernel security
setting was changed. Runtime checks verify `NoNewPrivs=1` and zero effective,
permitted, ambient, and inheritable capabilities in the actual daemon process.

The published 0.4.0 unit has the same startup defect on this guest. Its real
installer therefore failed to start the service. The upgrade test retained
that failure and verified that the new package recovered it. No workaround was
applied to the baseline archive, unit, or guest policy.

## Results

Passed in Ubuntu: release compilation, ELF dependency resolution, archive
installation fixtures, custom-prefix service startup, reinstallation with
identity preservation, and recovery upgrade from the published 0.4.0 package.
The old failed service had no generated native identity to migrate. The separate
reinstallation case supplies the live identity-preservation evidence.
All temporary test users and user managers were removed.

Passed on the host: Rust formatting, strict workspace Clippy, workspace tests,
Android debug build/lint/JVM unit tests, QML lint, all 49 JavaScript tests,
20 offscreen sharing scenarios, launcher tests, installer fixtures, and the
isolated debug systemd smoke test. Actionlint accepted both changed workflows.
The ordinary workspace suite still skips its environment-dependent ignored tests.

The Ubuntu-built tarball records glibc 2.39 and OpenSSL 3.0.13 in BUILD.txt.
This is local VM validation; the edited hosted workflows were not triggered.
Phone behavior, authenticated account migration, live desktop notification/media
interaction, and multicast discovery across machines remain unverified here.
