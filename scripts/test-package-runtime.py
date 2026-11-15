#!/usr/bin/env python3
"""Install and start release packages in disposable systemd user accounts.

Run as root only in a disposable VM or CI runner. Native port 24837 must be free.
No phone or account credentials are used.
"""

import argparse
import hashlib
import os
from pathlib import Path
import pwd
import shutil
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
import uuid


def command(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, timeout=45, **kwargs)


def output(*args):
    return command(*args, stdout=subprocess.PIPE).stdout.strip()


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def unpack(archive, directory):
    with tarfile.open(archive) as package:
        package.extractall(directory, filter="data")
    entries = list(directory.iterdir())
    assert len(entries) == 1 and entries[0].is_dir(), entries
    return entries[0]


def running(prefix, bundle):
    assert output("systemctl", "--user", "is-active", "handoverd.service") == "active"
    pid = int(output("systemctl", "--user", "show", "handoverd.service", "-p", "MainPID", "--value"))
    assert pid > 0
    assert Path(f"/proc/{pid}/exe").resolve() == prefix / "bin/handoverd"
    status = dict(line.split(":", 1) for line in Path(f"/proc/{pid}/status").read_text().splitlines())
    assert status["NoNewPrivs"].strip() == "1", "service can acquire new privileges"
    for name in ("CapEff", "CapPrm", "CapAmb", "CapInh"):
        assert int(status[name].strip(), 16) == 0, f"service has {name} capabilities"
    assert digest(prefix / "bin/handoverd") == digest(bundle / "bin/handoverd")
    fragment = Path(output("systemctl", "--user", "show", "handoverd.service", "-p", "FragmentPath", "--value"))
    assert fragment == Path(pwd.getpwuid(os.getuid()).pw_dir) / ".local/share/systemd/user/handoverd.service"
    runtime = Path(os.environ["XDG_RUNTIME_DIR"]) / "handover"
    assert runtime.stat().st_mode & 0o777 == 0o700
    assert (runtime / "handoverd.sock").stat().st_mode & 0o777 == 0o600
    command(str(prefix / "bin/handoverctl"), "devices")
    for _ in range(100):
        listeners = output("ss", "-H", "-ltnp", "sport = :24837")
        if f"pid={pid}," in listeners:
            break
        time.sleep(0.1)
    else:
        raise AssertionError("installed daemon does not own native TCP port 24837")
    # A listener must accept connections, not merely appear in systemd state.
    with socket.create_connection(("127.0.0.1", 24837), timeout=2):
        pass
    invocation = output("systemctl", "--user", "show", "handoverd.service", "-p", "InvocationID", "--value")
    logs = output("journalctl", "--user", f"_SYSTEMD_INVOCATION_ID={invocation}", "--no-pager", "-o", "cat")
    assert "native backend stopped" not in logs and "failed to start" not in logs, logs
    return invocation


def install(bundle, prefix, allow_legacy_capability_failure=False):
    result = subprocess.run([str(bundle / "install.sh"), "--without-google-messages-setup"],
                            env=dict(os.environ, PREFIX=str(prefix)), timeout=35, check=False)
    if result.returncode:
        status = output("systemctl", "--user", "show", "handoverd.service", "-p", "ExecMainStatus", "--value")
        if allow_legacy_capability_failure and result.returncode == 1 and status == "218":
            print("Baseline service failed with 218/CAPABILITIES; testing recovery by upgrade", flush=True)
            return output("systemctl", "--user", "show", "handoverd.service", "-p", "InvocationID", "--value")
        result.check_returncode()
    return running(prefix, bundle)


def account_test(archive, previous, case):
    account_directory = Path(pwd.getpwuid(os.getuid()).pw_dir)
    prefix = account_directory / ('apps/Handover 50% $cash "quote"' if case == "clean" else ".local")
    with tempfile.TemporaryDirectory(prefix="package-", dir=account_directory) as temporary:
        root = Path(temporary)
        current = unpack(archive, root / "current")
        for binary in (current / "bin").iterdir():
            if binary.read_bytes()[:4] == b"\x7fELF":
                linked = output("ldd", str(binary))
                assert "not found" not in linked, linked
        if case == "clean":
            install(current, prefix)
            print("PASS: clean install, custom service path, real CLI IPC, socket permissions, native TCP", flush=True)
            return
        baseline = unpack(previous, root / "previous") if case == "upgrade" else current
        first = install(baseline, prefix, allow_legacy_capability_failure=case == "upgrade")
        state = account_directory / ".local/state/handover"
        state.mkdir(parents=True, exist_ok=True)
        sentinel = state / "package-test-sentinel"
        sentinel.write_text("synthetic package upgrade state\n")
        identity = state / "native/identity.crt"
        original_identity = digest(identity) if identity.exists() else None
        if case == "reinstall":
            assert original_identity is not None, "current daemon did not persist its identity"
        # An obsolete client asset must be replaced by the upgrade.
        view = account_directory / ".local/share/handover/quickshell/HandoverService.qml"
        view.write_text("obsolete installed view\n")
        second = install(current, prefix)
        assert first != second, "upgrade did not start a new service invocation"
        assert sentinel.read_text() == "synthetic package upgrade state\n"
        if original_identity is not None:
            assert digest(identity) == original_identity, "upgrade replaced the native identity"
        assert digest(view) == digest(current / "share/handover/quickshell/HandoverService.qml")
        print(f"PASS: {case} started a new service invocation, replaced assets, preserved synthetic state"
              + (" and native identity" if original_identity else ""), flush=True)


def isolated_accounts(archive, previous):
    # Fail before account creation if this host already has a native daemon.
    with socket.socket() as probe:
        probe.bind(("0.0.0.0", 24837))
    with tempfile.TemporaryDirectory(prefix="handover-package-runtime-") as temporary:
        stage = Path(temporary)
        stage.chmod(0o755)
        runner = stage / "test-package-runtime.py"
        shutil.copyfile(__file__, runner)
        runner.chmod(0o644)
        current = stage / "current.tar.gz"
        shutil.copyfile(archive, current)
        current.chmod(0o644)
        baseline = None
        if previous:
            baseline = stage / "previous.tar.gz"
            shutil.copyfile(previous, baseline)
            baseline.chmod(0o644)
        for case in (("clean", "reinstall", "upgrade") if previous else ("clean", "reinstall")):
            user = "hpkg-" + uuid.uuid4().hex[:12]
            command("useradd", "--create-home", "--shell", "/bin/sh", user)
            uid = None
            try:
                account = pwd.getpwnam(user)
                uid = account.pw_uid
                environment = [f"XDG_RUNTIME_DIR=/run/user/{uid}",
                               f"DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/{uid}/bus",
                               f"XDG_CONFIG_HOME={account.pw_dir}/.config",
                               f"XDG_STATE_HOME={account.pw_dir}/.local/state",
                               f"XDG_DATA_HOME={account.pw_dir}/.local/share",
                               f"XDG_CACHE_HOME={account.pw_dir}/.cache"]
                command("loginctl", "enable-linger", user)
                command("systemctl", "start", f"user@{uid}.service")
                args = ["runuser", "-u", user, "--", "env", *environment,
                        "python3", str(runner), "--account-case", case, "--archive", str(current)]
                if baseline:
                    args.extend(["--previous-archive", str(baseline)])
                try:
                    command(*args)
                except (subprocess.SubprocessError, AssertionError):
                    # Keep the permission boundary visible before account cleanup.
                    for path in (Path(f"/run/user/{uid}"), Path(f"/run/user/{uid}/handover"),
                                 Path(f"/run/user/{uid}/handover/handoverd.sock")):
                        try:
                            metadata = path.lstat()
                            print(f"IPC metadata: {path} uid={metadata.st_uid} gid={metadata.st_gid} "
                                  f"mode={metadata.st_mode & 0o777:o}", flush=True)
                        except OSError as error:
                            print(f"IPC metadata: {path}: {error}", flush=True)
                    manager = subprocess.run(["runuser", "-u", user, "--", "env", *environment,
                                              "systemctl", "--user", "show-environment"],
                                             capture_output=True, text=True, timeout=15, check=False)
                    for line in manager.stdout.splitlines():
                        if line.startswith("XDG_RUNTIME_DIR="):
                            print(f"User manager: {line}", flush=True)
                    subprocess.run(["runuser", "-u", user, "--", "env", *environment,
                                    "journalctl", "--user", "-u", "handoverd.service", "--no-pager", "-n", "80"],
                                   timeout=15, check=False)
                    raise
            finally:
                test_failed = sys.exc_info()[0] is not None
                cleanup_errors = []
                stop = (["systemctl", "stop", f"user@{uid}.service"] if uid is not None
                        else ["loginctl", "terminate-user", user])
                for args in (["loginctl", "disable-linger", user], stop,
                             ["userdel", "--remove", user]):
                    try:
                        command(*args)
                    except (OSError, subprocess.SubprocessError) as error:
                        cleanup_errors.append(str(error))
                if cleanup_errors:
                    print("Temporary account cleanup failed: " + "; ".join(cleanup_errors), file=sys.stderr)
                    if not test_failed:
                        raise RuntimeError("temporary account cleanup failed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--previous-archive", type=Path,
                        help="earlier release archive; otherwise check reinstallation of the current build")
    parser.add_argument("--disposable-environment", action="store_true",
                        help="confirm this is a disposable VM or CI runner")
    parser.add_argument("--account-case", choices=("clean", "reinstall", "upgrade"), help=argparse.SUPPRESS)
    arguments = parser.parse_args()
    if arguments.account_case:
        if os.geteuid() == 0:
            parser.error("account checks must run as the disposable unprivileged user")
        account_test(arguments.archive, arguments.previous_archive, arguments.account_case)
    else:
        if os.geteuid() != 0 or not arguments.disposable_environment:
            parser.error("use root and --disposable-environment only in a disposable VM or CI runner")
        isolated_accounts(arguments.archive.resolve(),
                          arguments.previous_archive.resolve() if arguments.previous_archive else None)
