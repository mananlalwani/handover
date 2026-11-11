#!/usr/bin/env python3
"""Check installer files and upgrade behavior with isolated service/IPC fixtures."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tarfile
import tempfile
import threading
import tomllib
import uuid

ROOT = Path(__file__).resolve().parent.parent
VERSION = tomllib.loads((ROOT / "handoverd/Cargo.toml").read_text())["package"]["version"]
RUST_BINARIES = ("handoverd", "handover-desktop", "handover-messages", "handoverctl",
                 "handover-google-messages-helper", "handover-google-messages-auth-probe")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_assets():
    assets = {"packaging/handover.desktop": "share/applications/handover.desktop",
              "packaging/handover-send.desktop": "share/applications/handover-send.desktop",
              "packaging/handover-messages.desktop": "share/applications/handover-messages.desktop",
              "packaging/systemd/handoverd.local.service": "share/systemd/user/handoverd.service",
              "packaging/systemd/handover-desktop.local.service": "share/systemd/user/handover-desktop.service",
              "scripts/handover-gui": "bin/handover-gui",
              "scripts/handover-google-messages-setup": "bin/handover-google-messages-setup",
              "scripts/install-from-dist.sh": "install.sh",
              "docs/man/handoverctl.1": "share/man/man1/handoverctl.1",
              "docs/man/handoverd.8": "share/man/man8/handoverd.8",
              "completions/handoverctl.bash": "share/bash-completion/completions/handoverctl",
              "completions/_handoverctl": "share/zsh/site-functions/_handoverctl",
              "completions/handoverctl.fish": "share/fish/vendor_completions.d/handoverctl.fish",
              "LICENSE": "share/licenses/handover/LICENSE",
              "third_party/ukey2/LICENSE": "share/licenses/handover/ukey2/LICENSE",
              "third_party/ukey2/README.md": "share/licenses/handover/ukey2/README.md"}
    assets["scripts/install-service-unit.sh"] = "install-service-unit.sh"
    for path in [*(ROOT / "quickshell").iterdir(), *(ROOT / "quickshell/pages").iterdir()]:
        if path.is_file() and path.stem != "messages" and (path.suffix in (".qml", ".js") or path.name == "README.md"):
            assets[str(path.relative_to(ROOT))] = "share/handover/" + str(path.relative_to(ROOT))
    for name in ("setup_google_messages.py", "chromium_login_probe.py", "chromium_auth_capture.py",
                 "chromium_browsers.py", "install_native_probe.py"):
        assets[f"google-messages/tools/{name}"] = f"share/handover/google-messages/tools/{name}"
    for path in (ROOT / "google-messages/tools/chrome-observer").iterdir():
        if path.suffix in (".js", ".html", ".json") or path.name in ("README.md", "privacy.mjs", "wire-shape.mjs"):
            assets[str(path.relative_to(ROOT))] = "share/handover/" + str(path.relative_to(ROOT))
    return assets


def make_bundle(directory):
    for source, target in source_assets().items():
        destination = directory / target
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / source, destination)
    for name in RUST_BINARIES:
        path = directory / "bin" / name
        path.write_text("#!/bin/sh\nexit 0\n")
        path.chmod(0o755)
    # The fixture CLI really connects, so a stale socket cannot satisfy readiness.
    cli = directory / "bin/handoverctl"
    cli.write_text(f'''#!/usr/bin/env python3
import json, os, socket, sys
if sys.argv[1:] == ["--version"]:
    print("handoverctl {VERSION}")
    raise SystemExit(0)
assert sys.argv[1:] == ["devices"]
with socket.socket(socket.AF_UNIX) as connection:
    connection.connect(os.environ["XDG_RUNTIME_DIR"] + "/handover/handoverd.sock")
    connection.sendall(b'{{"protocol":1,"method":"devices.list"}}\\n')
    response = json.loads(connection.makefile("rb").readline())
    assert response["protocol"] == 1 and response["type"] == "devices"
''')


class IpcFixture:
    def __init__(self, path, stale=False, unresponsive=False):
        self.path = path
        self.stale = stale
        self.unresponsive = unresponsive
        self.stopped = threading.Event()
        self.errors = []

    def __enter__(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.listener = socket.socket(socket.AF_UNIX)
        self.listener.bind(str(self.path))
        if self.stale:
            self.listener.close()
            return self
        self.listener.listen(8)
        self.listener.settimeout(0.1)

        def run():
            while not self.stopped.is_set():
                try:
                    connection, _ = self.listener.accept()
                except socket.timeout:
                    continue
                try:
                    with connection:
                        connection.settimeout(2)
                        for line in connection.makefile("rb"):
                            request = json.loads(line)
                            if self.unresponsive:
                                continue
                            if request["method"] == "hello":
                                response = dict(protocol=1, type="hello", supported_protocols=[1])
                            else:
                                assert request["method"] == "devices.list", request
                                response = dict(protocol=1, type="devices", devices=[])
                            connection.sendall((json.dumps(response) + "\n").encode())
                except Exception as error:
                    self.errors.append(error)

        self.worker = threading.Thread(target=run, daemon=True)
        self.worker.start()
        return self

    def __exit__(self, *_):
        self.stopped.set()
        if not self.stale:
            self.worker.join(3)
            assert not self.worker.is_alive(), "IPC fixture did not terminate"
            self.listener.close()
            assert not self.errors, self.errors


def install(bundle, root, setup=False, stale=False, unusual=False, unresponsive=False):
    prefix = root / ('prefix with spaces ✓ 50% $cash "quote"' if unusual else "prefix")
    data = root / "data"
    runtime = root / "runtime"
    runtime.mkdir(parents=True, exist_ok=True)
    runtime.chmod(0o700)
    proxy = root / "fixture-bin"
    proxy.mkdir(exist_ok=True)
    log = root / "service-actions.jsonl"
    systemctl = proxy / "systemctl"
    systemctl.write_text('''#!/usr/bin/env python3
import json, os, sys
allowed = [["--user", "daemon-reload"], ["--user", "reenable", "handoverd.service"],
           ["--user", "restart", "handoverd.service"]]
assert sys.argv[1:] in allowed, sys.argv
with open(os.environ["HANDOVER_INSTALL_TEST_LOG"], "a") as output:
    output.write(json.dumps(sys.argv[1:]) + "\\n")
''')
    systemctl.chmod(0o755)
    environment = dict(os.environ, PREFIX=str(prefix), XDG_DATA_HOME=str(data),
                       XDG_RUNTIME_DIR=str(runtime), XDG_STATE_HOME=str(root / "state"),
                       XDG_CONFIG_HOME=str(root / "config"), XDG_CACHE_HOME=str(root / "cache"),
                       PATH=str(proxy) + os.pathsep + os.environ["PATH"], HANDOVER_INSTALL_TEST_LOG=str(log))
    with IpcFixture(runtime / "handover/handoverd.sock", stale, unresponsive):
        process = subprocess.run([str(bundle / "install.sh"),
                                  "--with-google-messages-setup" if setup else "--without-google-messages-setup"],
                                 env=environment, capture_output=True, text=True, timeout=35)
    (runtime / "handover/handoverd.sock").unlink()
    if stale or unresponsive:
        assert process.returncode != 0, "installer accepted a socket without a responding daemon"
        assert "did not respond over IPC" in process.stderr, process.stderr
        return prefix, data
    assert process.returncode == 0, process.stdout + process.stderr
    calls = [json.loads(line) for line in log.read_text().splitlines()]
    assert calls[-3:] == [["--user", "daemon-reload"], ["--user", "reenable", "handoverd.service"],
                          ["--user", "restart", "handoverd.service"]], calls
    return prefix, data


def check_files(bundle, prefix, data, setup):
    for path in (bundle / "bin").iterdir():
        if path.name == "handover-google-messages-setup" and not setup:
            assert not (prefix / "bin" / path.name).exists()
            continue
        installed = prefix / "bin" / path.name
        assert installed.is_file() and os.access(installed, os.X_OK), installed
        assert digest(installed) == digest(path), installed
    for path in (bundle / "share").rglob("*"):
        if not path.is_file() or path.suffix == ".service":
            continue
        relative = path.relative_to(bundle / "share")
        if not setup and relative.name in ("setup_google_messages.py", "chromium_login_probe.py",
                                            "chromium_auth_capture.py", "chromium_browsers.py"):
            continue
        destination = (prefix / "share" / relative if relative.parts[0] in ("man", "licenses")
                       else data / relative)
        assert destination.is_file(), destination
        assert digest(destination) == digest(path), destination
    for name in ("handoverd", "handover-desktop"):
        unit = data / "systemd/user" / f"{name}.service"
        line = next(line for line in unit.read_text().splitlines() if line.startswith("ExecStart="))
        # Check the path contract independently of the renderer; systemd parses tricky paths below.
        if " " not in str(prefix):
            assert line == f'ExecStart=/usr/bin/env -- "{prefix}/bin/{name}"', line
        assert "NoNewPrivileges=yes" in unit.read_text(), unit
        original = bundle / "share/systemd/user" / f"{name}.service"
        assert [line for line in unit.read_text().splitlines() if not line.startswith("ExecStart=")] == [
            line for line in original.read_text().splitlines() if not line.startswith("ExecStart=")], unit
    if shutil.which("systemd-analyze"):
        checked = subprocess.run(["systemd-analyze", "--user", "verify", "--man=no",
                                  str(data / "systemd/user/handoverd.service"),
                                  str(data / "systemd/user/handover-desktop.service")],
                                 env=dict(os.environ, XDG_RUNTIME_DIR=str(prefix.parent / "runtime"),
                                          XDG_CONFIG_HOME=str(prefix.parent / "config")),
                                 capture_output=True, text=True, timeout=15)
        assert checked.returncode == 0, checked.stdout + checked.stderr
    # Exercise the rendered command with harmless fixture binaries only. Never
    # start an artifact's real daemon here or load the installed service names.
    executable = prefix / "bin/handoverd"
    if executable.read_bytes() == b"#!/bin/sh\nexit 0\n" and shutil.which("systemd-run"):
        manager = subprocess.run(["systemctl", "--user", "show-environment"],
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if manager.returncode == 0:
            original = executable.read_bytes()
            marker = prefix.parent / "rendered-command"
            executable.write_text('#!/bin/sh\nprintf "%s\\n" "$0" > "$HANDOVER_UNIT_MARKER"\n')
            start = next(line for line in (data / "systemd/user/handoverd.service").read_text().splitlines()
                         if line.startswith("ExecStart="))
            try:
                # Transient ExecStart properties take specifiers already expanded;
                # the unit-file parser's literal %% is a single % at that stage.
                checked = subprocess.run(["systemd-run", "--user", "--wait", "--collect", "--pipe",
                                          "--service-type=oneshot", "--unit=handover-install-" + uuid.uuid4().hex,
                                          "--setenv=HANDOVER_UNIT_MARKER=" + str(marker),
                                          "--property=" + start.replace("%%", "%"), "/usr/bin/true"],
                                         capture_output=True, text=True, timeout=15)
                assert checked.returncode == 0, checked.stdout + checked.stderr
                assert marker.read_text().strip() == str(executable), marker.read_text()
            finally:
                executable.write_bytes(original)
    assert (prefix / "bin/handover-gui.data-home").read_text().strip() == str(data)
    version = subprocess.check_output([str(prefix / "bin/handoverctl"), "--version"], text=True).strip()
    assert version == f"handoverctl {VERSION}", version
    desktop = data / "applications/handover-messages.desktop"
    assert "Exec=handover-messages" in desktop.read_text()


def run(bundle, root, case, previous=None):
    if case in ("all", "custom-prefix"):
        directory = root / "clean"
        directory.mkdir()
        prefix, data = install(bundle, directory)
        check_files(bundle, prefix, data, False)
    if case in ("all", "stale-socket"):
        directory = root / "stale"
        directory.mkdir()
        install(bundle, directory, stale=True)
    if case == "all":
        directory = root / "unresponsive"
        directory.mkdir()
        install(bundle, directory, unresponsive=True)
        directory = root / "unusual"
        directory.mkdir()
        prefix, data = install(bundle, directory, unusual=True)
        check_files(bundle, prefix, data, False)
        directory = root / "upgrade"
        directory.mkdir()
        prefix, data = install(bundle, directory, setup=True)
        check_files(bundle, prefix, data, True)
        sentinel = directory / "state/keep-state"
        sentinel.parent.mkdir(parents=True, exist_ok=True)
        sentinel.write_bytes(b"synthetic saved state")
        (prefix / "bin/handoverd").write_text("obsolete executable")
        (data / "handover/quickshell/HandoverService.qml").write_text("obsolete view")
        (data / "handover/google-messages/tools/chromium_browsers.py").write_text("obsolete setup module")
        prefix, data = install(bundle, directory, setup=True)
        check_files(bundle, prefix, data, True)
        assert sentinel.read_bytes() == b"synthetic saved state"
        if previous:
            directory = root / "previous-version"
            directory.mkdir()
            prefix, data = install(previous, directory, setup=True)
            state = directory / "state/keep-state"
            state.parent.mkdir(parents=True, exist_ok=True)
            state.write_bytes(b"synthetic prior-version state")
            prefix, data = install(bundle, directory, setup=True)
            check_files(bundle, prefix, data, True)
            assert state.read_bytes() == b"synthetic prior-version state"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--previous-archive", type=Path)
    parser.add_argument("--case", choices=("all", "custom-prefix", "stale-socket"), default="all")
    arguments = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="handover-install-") as temporary:
        root = Path(temporary)
        if arguments.archive:
            with tarfile.open(arguments.archive) as archive:
                archive.extractall(root / "unpacked", filter="data")
            entries = list((root / "unpacked").iterdir())
            assert len(entries) == 1 and entries[0].is_dir(), entries
            bundle = entries[0]
            for name in RUST_BINARIES:
                binary = bundle / "bin" / name
                assert binary.read_bytes()[:4] == b"\x7fELF", binary
                linked = subprocess.run(["ldd", str(binary)], capture_output=True, text=True, timeout=10)
                assert linked.returncode == 0 and "not found" not in linked.stdout, linked.stdout + linked.stderr
            for source, target in source_assets().items():
                assert (bundle / target).is_file(), target
                assert digest(bundle / target) == digest(ROOT / source), target
        else:
            bundle = root / "bundle"
            bundle.mkdir()
            make_bundle(bundle)
        previous = None
        if arguments.previous_archive:
            with tarfile.open(arguments.previous_archive) as archive:
                archive.extractall(root / "previous", filter="data")
            entries = list((root / "previous").iterdir())
            assert len(entries) == 1 and entries[0].is_dir(), entries
            previous = entries[0]
        run(bundle, root, arguments.case, previous)
    print("Installer checks passed with isolated service and IPC fixtures")
