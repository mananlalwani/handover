#!/usr/bin/env python3
"""Run the native Messages window test on a private display and D-Bus."""

import json
import os
from pathlib import Path
import selectors
import shutil
import signal
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def stop_group(process):
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=5)
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def read_line(process, description):
    with selectors.DefaultSelector() as selector:
        selector.register(process.stdout, selectors.EVENT_READ)
        if not selector.select(timeout=8):
            raise RuntimeError(f"{description} did not start")
        return process.stdout.readline().strip()


def main():
    for tool in ("cargo", "dbus-daemon", "Xvfb"):
        if not shutil.which(tool):
            raise SystemExit(f"Required tool is missing: {tool}")
    built = subprocess.run(
        ["cargo", "test", "--locked", "-p", "handover-messages", "--bin",
         "handover-messages", "--no-run", "--message-format=json"],
        cwd=ROOT, check=True, text=True, stdout=subprocess.PIPE,
    )
    binaries = [entry["executable"] for line in built.stdout.splitlines()
                if (entry := json.loads(line)).get("reason") == "compiler-artifact"
                and entry.get("executable") and entry["target"].get("name") == "handover-messages"
                and entry["target"].get("test")]
    if len(binaries) != 1:
        raise SystemExit("native Messages GUI test executable not found")
    application_build = subprocess.run(
        ["cargo", "build", "--locked", "-p", "handover-messages", "--bin",
         "handover-messages", "--message-format=json"],
        cwd=ROOT, check=True, text=True, stdout=subprocess.PIPE,
    )
    applications = [entry["executable"] for line in application_build.stdout.splitlines()
                    if (entry := json.loads(line)).get("reason") == "compiler-artifact"
                    and entry.get("executable") and entry["target"].get("name") == "handover-messages"
                    and "bin" in entry["target"].get("kind", [])]
    if len(applications) != 1:
        raise SystemExit("native Messages application executable not found")

    with tempfile.TemporaryDirectory(prefix="handover-messages-ui-") as temporary:
        root = Path(temporary)
        environment = dict(os.environ, HANDOVER_MESSAGES_TEST_ROOT=str(root))
        environment["GDK_BACKEND"] = "x11"
        environment["GTK_A11Y"] = "none"
        environment["HANDOVER_MESSAGES_TEST_BINARY"] = applications[0]
        for name in ("DATA_HOME", "CONFIG_HOME", "STATE_HOME", "CACHE_HOME"):
            path = root / name.lower()
            path.mkdir(mode=0o700)
            environment["XDG_" + name] = str(path)
        (root / "runtime").mkdir(mode=0o700)
        environment["XDG_RUNTIME_DIR"] = str(root / "runtime")
        fake_bin = root / "bin"
        fake_bin.mkdir(mode=0o700)
        fake_helper = fake_bin / "handover-google-messages-setup"
        fake_helper.write_text('''#!/bin/sh
if [ "$1" = "--list-browsers" ]; then
  printf '%s\\n' '{"browsers":[{"name":"Fake Chromium","path":"/bin/true"}]}'
else
  printf '%s\\n' '{"status":"progress","message":"Fake setup progress"}'
  sleep 0.4
  printf '%s\\n' '{"status":"saved","message":"Fake setup saved"}'
fi
''')
        fake_helper.chmod(0o700)
        environment["PATH"] = str(fake_bin) + os.pathsep + environment.get("PATH", "")
        (root / "runtime/handover").mkdir(mode=0o700)
        for name in ("DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS",
                     "DBUS_STARTER_ADDRESS", "DBUS_STARTER_BUS_TYPE"):
            environment.pop(name, None)

        bus_config = root / "bus.conf"
        bus_config.write_text(f'''<busconfig>
  <type>session</type>
  <listen>unix:path={root}/runtime/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*"/>
    <allow receive_sender="*"/>
    <allow own="*"/>
  </policy>
</busconfig>
''')
        bus = subprocess.Popen(["dbus-daemon", "--nofork", "--print-address=1",
                                "--config-file=" + str(bus_config)],
                               stdout=subprocess.PIPE, text=True, start_new_session=True)
        display = None
        test = None
        try:
            address = read_line(bus, "private D-Bus")
            if not address.startswith(f"unix:path={root}/runtime/bus"):
                raise RuntimeError("private D-Bus returned an unexpected address")
            environment["DBUS_SESSION_BUS_ADDRESS"] = address
            display = subprocess.Popen(["Xvfb", "-displayfd", "1", "-screen", "0",
                                        "1280x900x24", "-nolisten", "tcp"],
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                       text=True, start_new_session=True)
            display_number = read_line(display, "private Xvfb")
            environment["DISPLAY"] = ":" + display_number
            test = subprocess.Popen(
                [binaries[0], "--ignored", "--exact",
                 "ui::tests::native_window_round_trips_real_frontend_actions", "--nocapture"],
                env=environment, start_new_session=True,
            )
            test.wait(timeout=120)
            if test.returncode:
                raise SystemExit("native Messages GUI integration failed")
        finally:
            if test is not None:
                stop_group(test)
            if display is not None:
                stop_group(display)
            stop_group(bus)


if __name__ == "__main__":
    main()
