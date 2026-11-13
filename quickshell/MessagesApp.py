#!/usr/bin/env python3
"""Find, install, and launch the standalone Messages app for the current user."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

APP = "handover-messages"
MAX_BINARY_BYTES = 256 * 1024 * 1024


def installed_binary():
    candidates = [Path.home() / ".local/bin" / APP]
    installation_bin = os.environ.get("HANDOVER_BIN_DIR")
    if installation_bin and Path(installation_bin).is_absolute():
        candidates.append(Path(installation_bin) / APP)
    found = shutil.which(APP)
    if found:
        candidates.append(Path(found))
    for candidate in candidates:
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return candidate.resolve()
    return None


def atomic_write(destination, writer, mode):
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as output:
            temporary = Path(output.name)
            writer(output)
            output.flush()
            os.fchmod(output.fileno(), mode)
        os.replace(temporary, destination)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def desktop_string(value):
    return str(value).replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t")


def desktop_argument(path):
    # Quote one Exec argument, then apply the Desktop Entry string escape layer.
    # Percent is an Exec field code, even inside quotes.
    escaped = str(path).replace("%", "%%")
    for character in ('\\', '"', '`', '$'):
        escaped = escaped.replace(character, '\\' + character)
    return desktop_string('"' + escaped + '"')


def desktop_exec(path):
    # A fixed executable avoids desktop parsers treating field codes in the
    # executable's own name as part of discovery. The path is a positional
    # argument, never shell code.
    return "/bin/sh -c " + desktop_argument('exec "$1"') + " handover-messages " + desktop_argument(path)


def install(source):
    source = Path(source).expanduser().resolve(strict=True)
    if not source.is_file() or not 4 <= source.stat().st_size <= MAX_BINARY_BYTES:
        raise ValueError("Choose the handover-messages executable from an extracted Linux release or your own build.")
    with source.open("rb") as binary:
        if binary.read(4) != b"\x7fELF":
            raise ValueError("Choose bin/handover-messages, not the release archive or desktop shortcut.")
        binary.seek(0)
        destination = Path.home() / ".local/bin" / APP
        atomic_write(destination, lambda output: shutil.copyfileobj(binary, output), 0o755)
    data_home = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")
    desktop = ("[Desktop Entry]\nType=Application\nName=Handover Messages\n"
               "Comment=Messages from your connected accounts\n"
               f"TryExec={desktop_string(destination)}\nExec={desktop_exec(destination)}\nIcon=mail-message-new\n"
               "Terminal=false\nCategories=Network;InstantMessaging;\n")
    atomic_write(data_home / "applications/handover-messages.desktop",
                 lambda output: output.write(desktop.encode()), 0o644)
    return destination


def launch(binary):
    process = subprocess.Popen([str(binary)], stdin=subprocess.DEVNULL,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                               start_new_session=True, close_fds=True)
    try:
        code = process.wait(timeout=0.5)
    except subprocess.TimeoutExpired:
        return
    if code != 0:
        raise ValueError("Handover Messages exited during startup. Check that GTK 4.10 and libadwaita 1.4 or newer are installed, or use the QML fallback.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("status", "install", "launch"))
    parser.add_argument("source", nargs="?")
    args = parser.parse_args()
    try:
        binary = installed_binary()
        if args.action == "install":
            if not args.source:
                raise ValueError("Choose an app executable first.")
            binary = install(args.source)
        elif args.action == "launch":
            if binary is None:
                raise ValueError("Handover Messages is not installed. Install it or use the QML fallback.")
            launch(binary)
        print(json.dumps({"success": True, "installed": binary is not None}))
        return 0
    except (OSError, ValueError) as error:
        print(json.dumps({"success": False, "installed": installed_binary() is not None,
                          "error": str(error)}))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
