#!/usr/bin/env python3
"""Sign-in-only experiment using a disposable Chromium profile and private pipe."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import select
import shutil
import signal
import tempfile
import time


def find_browser(explicit):
    candidates = [explicit] if explicit else [
        shutil.which("chromium"), shutil.which("chromium-browser"),
        shutil.which("google-chrome"), "/opt/helium-browser-bin/chrome",
    ]
    for candidate in candidates:
        if candidate and Path(candidate).is_file() and os.access(candidate, os.X_OK):
            return str(Path(candidate).resolve())
    raise RuntimeError("No Chromium browser found; supply --browser with its executable path")


def private_fd(fd):
    # Keep spawn sources away from Chromium's reserved pipe descriptors 3 and 4.
    duplicate = fcntl.fcntl(fd, fcntl.F_DUPFD_CLOEXEC, 10)
    os.close(fd)
    return duplicate


def run(browser, smoke):
    url = "about:blank" if smoke else (
        "https://accounts.google.com/AccountChooser?continue="
        "https%3A%2F%2Fmessages.google.com%2Fweb%2Fconfig"
    )
    with tempfile.TemporaryDirectory(prefix="handover-chromium-signin-") as profile:
        read_in, write_in = map(private_fd, os.pipe())
        read_out, write_out = map(private_fd, os.pipe())
        null = private_fd(os.open(os.devnull, os.O_RDWR))
        descriptors = [read_in, write_in, read_out, write_out, null]
        actions = [(os.POSIX_SPAWN_DUP2, read_in, 3),
                   (os.POSIX_SPAWN_DUP2, write_out, 4)]
        actions += [(os.POSIX_SPAWN_DUP2, null, fd) for fd in (0, 1, 2)]
        actions += [(os.POSIX_SPAWN_CLOSE, fd) for fd in descriptors]
        args = [browser, "--disable-sync", "--no-first-run", "--no-default-browser-check",
                "--remote-debugging-pipe", f"--user-data-dir={profile}", f"--app={url}"]
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
            print("Private browser pipe ready. No authentication was captured.", flush=True)
            if smoke:
                return
            print("Sign in, then close the window. Do not open the conversation list.", flush=True)
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
    args = parser.parse_args()
    try:
        run(find_browser(args.browser), args.self_test)
    except (OSError, RuntimeError, ValueError):
        # Browser output and arbitrary error values may contain private URLs.
        print("Chromium sign-in probe failed. Check browser availability and display access.")
        return 1
    except KeyboardInterrupt:
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
