#!/usr/bin/env python3
"""Connect Google Messages using an installed Chromium browser and native client."""

import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import struct
import subprocess

from chromium_browsers import discover_browsers
from chromium_login_probe import run as browser_sign_in


def emit(message, status="progress", **fields):
    print(json.dumps({"status": status, "message": message, **fields}), flush=True)


def cancel_setup(_signal, _frame):
    raise KeyboardInterrupt()


def daemon_request(method, **fields):
    runtime = Path(os.environ.get("XDG_RUNTIME_DIR", ""))
    if not runtime.is_absolute():
        raise RuntimeError("Daemon runtime unavailable")
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.settimeout(20)
        client.connect(str(runtime / "handover/handoverd.sock"))
        client.sendall(json.dumps({"protocol": 1, "method": method, **fields}).encode() + b"\n")
        response = client.makefile("rb").readline(4097)
    if len(response) > 4096 or not response.endswith(b"\n"):
        raise RuntimeError("Invalid setup response")
    value = json.loads(response)
    if value.get("protocol") != 1 or value.get("type") == "error":
        raise RuntimeError("Messaging setup unavailable or already in progress")
    return value


class MessagingPause:
    def __init__(self):
        self.lease = None

    def __enter__(self):
        result = daemon_request("messages.setup.begin")
        if (result.get("type") != "messaging_setup_paused"
                or not isinstance(result.get("lease"), str)
                or len(result["lease"]) > 128):
            raise RuntimeError("Messaging setup pause failed")
        self.lease = result["lease"]
        return self

    def __exit__(self, *_):
        if self.lease:
            result = daemon_request("messages.setup.end", lease=self.lease)
            self.lease = None
            if result.get("type") != "messaging_setup_resumed":
                raise RuntimeError("Messaging receiver restoration failed")


def native_setup(native, proof):
    payload = json.dumps(proof).encode()
    if len(payload) > 32 * 1024:
        raise RuntimeError("Setup proof exceeds bound")
    process = subprocess.Popen([native, "--local-setup"], stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                               start_new_session=True)
    saved = False
    try:
        process.stdin.write(struct.pack("=I", len(payload)) + payload)
        process.stdin.close()
        payload = b""
        proof.clear()
        while True:
            line = process.stdout.readline(1025)
            if not line:
                break
            if len(line) > 1024 or not line.endswith(b"\n"):
                raise RuntimeError("Invalid native setup status")
            event = json.loads(line)
            status, message = event.get("status"), event.get("message")
            if status not in ("progress", "failed", "saved") or not isinstance(message, str) or len(message) > 512:
                raise RuntimeError("Invalid native setup status")
            # These records contain fixed status text and the public pairing symbol.
            emit(message, status, **({"account": event["account"]} if status == "saved" else {}))
            saved = status == "saved"
        if process.wait(timeout=5) != 0 or not saved:
            raise RuntimeError("Native setup did not complete")
    finally:
        proof.clear()
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--list-browsers", action="store_true")
    parser.add_argument("--browser", help="Installed Chromium-compatible executable")
    parser.add_argument("--native-probe", help=argparse.SUPPRESS)
    args = parser.parse_args()
    signal.signal(signal.SIGTERM, cancel_setup)
    proof = None
    stage = "prepare"
    try:
        browsers = discover_browsers(args.browser)
        if args.list_browsers:
            print(json.dumps({"browsers": [{"name": item.name, "path": item.path} for item in browsers]}), flush=True)
            return 0
        if not browsers:
            emit("Install a Chromium browser, then try Google Messages setup again.", "failed")
            return 1
        native = args.native_probe or shutil.which("handover-google-messages-auth-probe")
        if not native or not Path(native).is_file() or not os.access(native, os.X_OK):
            emit("The native Google Messages client is not installed.", "failed")
            return 1
        emit("Preparing Google Messages setup. Other Handover features remain available.")
        with MessagingPause():
            stage = "signin"
            proof = browser_sign_in(browsers[0].path, False, setup=True, notify=emit)
            # browser_sign_in closes the owned browser and deletes its profile
            # before registration or phone pairing begins.
            stage = "native"
            native_setup(native, proof)
        emit("Messaging receiver resumed. Handover will show the account when it is online.", "resumed")
        return 0
    except KeyboardInterrupt:
        emit("Setup cancelled. Check phone pairing state before trying again.", "cancelled")
        return 130
    except (OSError, RuntimeError, ValueError, KeyError, subprocess.SubprocessError):
        messages = {
            "prepare": "Google Messages setup is unavailable or already running. Check Handover and try again.",
            "signin": "Browser sign-in could not be completed. Native pairing was not started. "
                      "You can choose another installed browser and try again.",
            "native": "Setup did not complete. Check phone pairing state before trying again.",
        }
        emit(messages[stage] + " The messaging receiver resumes automatically if immediate restoration fails.", "failed")
        return 1
    finally:
        if proof is not None:
            proof.clear()


if __name__ == "__main__":
    raise SystemExit(main())
