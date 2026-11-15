#!/usr/bin/env python3
"""Run credential-store integration against GNOME Keyring on a private D-Bus."""

import os
from pathlib import Path
import selectors
import shutil
import signal
import subprocess
import tempfile
from cargo_test_artifacts import test_executable

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
    # Also stop descendants if the test exited before their cleanup completed.
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def main():
    for tool in ("cargo", "dbus-daemon", "gnome-keyring-daemon"):
        if not shutil.which(tool):
            raise SystemExit(f"Required tool is missing: {tool}")
    binary = test_executable(
        "desktop_credential_store",
        ["cargo", "test", "--locked", "-p", "handover-google-messages", "--test",
         "desktop_credential_store", "--no-run", "--message-format=json"],
        ROOT,
    )
    with tempfile.TemporaryDirectory(prefix="handover-keyring-") as temporary:
        root = Path(temporary)
        environment = dict(os.environ, HANDOVER_SECRET_SERVICE_TEST_ROOT=str(root))
        for name in ("DATA_HOME", "CONFIG_HOME", "STATE_HOME", "CACHE_HOME", "RUNTIME_DIR"):
            path = root / name.lower()
            path.mkdir(mode=0o700)
            environment["XDG_" + name] = str(path)
        # GNOME Keyring can fall back to ~/.gnome2/keyrings if this is absent.
        (root / "data_home/keyrings").mkdir(mode=0o700)
        for name in ("GNOME_KEYRING_CONTROL", "GNOME_KEYRING_PID", "DISPLAY", "WAYLAND_DISPLAY"):
            environment.pop(name, None)
        configuration = root / "bus.conf"
        configuration.write_text(f'''<busconfig>
  <type>session</type>
  <listen>unix:path={root}/runtime_dir/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*"/>
    <allow receive_sender="*"/>
    <allow own="*"/>
  </policy>
</busconfig>
''')
        # No service directories: this bus cannot activate desktop services.
        bus = subprocess.Popen(["dbus-daemon", "--nofork", "--print-address=1",
                                "--config-file=" + str(configuration)],
                               stdout=subprocess.PIPE, text=True, start_new_session=True)
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(bus.stdout, selectors.EVENT_READ)
                assert selector.select(timeout=5), "private D-Bus did not start"
                address = bus.stdout.readline().strip()
            assert address.startswith(f"unix:path={root}/runtime_dir/bus"), "unexpected private bus address"
            environment["DBUS_SESSION_BUS_ADDRESS"] = address
            test = subprocess.Popen([binary, "--ignored", "--exact", "desktop_store_lifecycle",
                                     "--nocapture"], env=environment, start_new_session=True)
            try:
                test.wait(timeout=90)
                assert test.returncode == 0, "real Secret Service integration failed"
            finally:
                stop_group(test)
        finally:
            stop_group(bus)


if __name__ == "__main__":
    main()
