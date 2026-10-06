#!/usr/bin/env python3
"""Sign-in-only experiment using a disposable Chromium profile and private pipe."""

import argparse
import fcntl
import json
import os
import select
import signal
import subprocess
import tempfile
import time

from chromium_auth_capture import capture, capture_proof
from chromium_browsers import discover_browsers


def find_browser(explicit):
    candidates = discover_browsers(explicit)
    if candidates:
        return candidates[0].path
    raise RuntimeError("No Chromium browser found; supply --browser with its executable path")


def private_fd(fd):
    # Keep spawn sources away from Chromium's reserved pipe descriptors 3 and 4.
    duplicate = fcntl.fcntl(fd, fcntl.F_DUPFD_CLOEXEC, 10)
    os.close(fd)
    return duplicate


def normal_sign_in(browser, profile, notify=print):
    args = [browser, f"--user-data-dir={profile}", "--no-first-run",
            "--no-default-browser-check", "https://accounts.google.com/"]
    process = subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, start_new_session=True)
    notify("Sign in normally, then choose the browser menu's Exit to continue.")
    try:
        if process.wait(timeout=7 * 60) != 0:
            raise RuntimeError("Normal browser exited unsuccessfully")
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()


def run(browser, smoke, native_probe=None, setup=False, notify=print):
    url = "about:blank" if smoke or native_probe or setup else "https://messages.google.com/web/config"
    with tempfile.TemporaryDirectory(prefix="handover-chromium-signin-") as profile:
        if not smoke:
            normal_sign_in(browser, profile, notify)
        read_in, write_in = map(private_fd, os.pipe())
        read_out, write_out = map(private_fd, os.pipe())
        null = private_fd(os.open(os.devnull, os.O_RDWR))
        descriptors = [read_in, write_in, read_out, write_out, null]
        actions = [(os.POSIX_SPAWN_DUP2, read_in, 3),
                   (os.POSIX_SPAWN_DUP2, write_out, 4)]
        actions += [(os.POSIX_SPAWN_DUP2, null, fd) for fd in (0, 1, 2)]
        actions += [(os.POSIX_SPAWN_CLOSE, fd) for fd in descriptors]
        args = [browser, "--no-first-run", "--no-default-browser-check",
                "--remote-debugging-pipe", f"--user-data-dir={profile}", url]
        pid = None
        reaped = False
        try:
            pid = os.posix_spawn(browser, args, os.environ, file_actions=actions, setsid=True)
            for fd in (read_in, write_out, null):
                os.close(fd)
                descriptors.remove(fd)
            os.write(write_in, b'{"id":1,"method":"Browser.getVersion"}\0')
            deadline = time.monotonic() + 20
            pending = b""
            ready = False
            while time.monotonic() < deadline and not ready:
                if not select.select([read_out], [], [], 0.5)[0]:
                    continue
                chunk = os.read(read_out, 65536)
                if not chunk:
                    break
                pending += chunk
                if len(pending) > 1024 * 1024:
                    raise RuntimeError("Unexpected browser response size")
                while b"\0" in pending:
                    message, pending = pending.split(b"\0", 1)
                    response = json.loads(message)
                    if response.get("id") == 1:
                        ready = "result" in response
            if not ready:
                raise RuntimeError("Browser private pipe did not become ready")
            if not setup:
                print("Private browser pipe ready. No authentication was captured.", flush=True)
            if smoke:
                return
            if setup:
                notify("Finishing browser sign-in. Keep your phone available.")
                return capture_proof(read_out, write_in, include_account=True)
            if native_probe:
                capture(read_out, write_in, native_probe)
                return
            print("Check whether the configuration page remains signed in, then close the window. "
                  "Do not open the conversation list.", flush=True)
            deadline = time.monotonic() + 600
            while time.monotonic() < deadline:
                if os.waitpid(pid, os.WNOHANG)[0]:
                    reaped = True
                    break
                time.sleep(0.25)
        finally:
            if pid is not None and not reaped:
                # End the owned browser group before removing its temporary profile.
                try:
                    os.killpg(pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                for _ in range(40):
                    if os.waitpid(pid, os.WNOHANG)[0]:
                        reaped = True
                        break
                    time.sleep(0.05)
                if not reaped:
                    os.killpg(pid, signal.SIGKILL)
                    os.waitpid(pid, 0)
            for fd in descriptors:
                os.close(fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--browser", help="Installed Chromium-compatible executable")
    parser.add_argument("--self-test", action="store_true", help="Open only about:blank and exit")
    parser.add_argument("--read-only-auth-probe", metavar="PATH",
                        help="Run one native read-only check after sign-in; pause the existing receiver first")
    args = parser.parse_args()
    if args.self_test and args.read_only_auth_probe:
        parser.error("--self-test cannot capture authentication")
    try:
        run(find_browser(args.browser), args.self_test, args.read_only_auth_probe)
    except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired):
        # Browser output and arbitrary error values may contain private URLs.
        print("Chromium setup or its read-only authentication check could not complete.")
        return 1
    except KeyboardInterrupt:
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
