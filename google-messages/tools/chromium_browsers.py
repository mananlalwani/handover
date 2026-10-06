"""Discover installed Chromium-compatible browsers without starting them."""

from dataclasses import dataclass
import os
from pathlib import Path
import shlex
import shutil
import subprocess


@dataclass(frozen=True)
class BrowserCandidate:
    name: str
    path: str


_PROGRAMS = (
    ("Google Chrome", ("google-chrome-stable", "google-chrome", "chrome")),
    ("Chromium", ("chromium", "chromium-browser")),
    ("Brave", ("brave-browser", "brave")),
    ("Microsoft Edge", ("microsoft-edge-stable", "microsoft-edge", "msedge")),
    ("Vivaldi", ("vivaldi-stable", "vivaldi")),
    ("Opera", ("opera",)),
    ("Helium", ("helium",)),
)
_DEFAULT_EXECUTABLES = {
    "google-chrome": "Google Chrome",
    "google-chrome-stable": "Google Chrome",
    "chrome": "Google Chrome",
    "chromium": "Chromium",
    "chromium-browser": "Chromium",
    "brave": "Brave",
    "brave-browser": "Brave",
    "brave-browser-stable": "Brave",
    "microsoft-edge": "Microsoft Edge",
    "microsoft-edge-stable": "Microsoft Edge",
    "msedge": "Microsoft Edge",
    "vivaldi": "Vivaldi",
    "vivaldi-stable": "Vivaldi",
    "opera": "Opera",
    "helium": "Helium",
    "helium-browser": "Helium",
    "helium-wrapper": "Helium",
}
_KNOWN_PATHS = (
    ("Google Chrome", ("/opt/google/chrome/chrome", "/usr/lib/google-chrome/chrome")),
    ("Chromium", ("/usr/lib/chromium/chromium", "/usr/lib/chromium-browser/chromium-browser")),
    ("Brave", ("/opt/brave.com/brave/brave", "/usr/lib/brave-browser/brave")),
    ("Microsoft Edge", ("/opt/microsoft/msedge/msedge", "/usr/lib/microsoft-edge/msedge")),
    ("Vivaldi", ("/opt/vivaldi/vivaldi", "/usr/lib/vivaldi/vivaldi")),
    ("Opera", ("/usr/lib/opera/opera", "/usr/lib/x86_64-linux-gnu/opera/opera")),
    ("Helium", ("/opt/helium-browser-bin/chrome", "/opt/helium/chrome")),
)


def _executable(path):
    if not path:
        return None
    candidate = Path(path).expanduser()
    if candidate.is_file() and os.access(candidate, os.X_OK):
        return str(candidate.resolve())
    return None


def _data_dirs(env):
    home = env.get("XDG_DATA_HOME") or str(Path(env.get("HOME", "~")).expanduser() / ".local/share")
    dirs = [home]
    dirs.extend((env.get("XDG_DATA_DIRS") or "/usr/local/share:/usr/share").split(":"))
    return [Path(item) / "applications" for item in dirs if item]


def _default_exec(env, which=shutil.which, run=subprocess.run):
    try:
        result = run(["xdg-settings", "get", "default-web-browser"],
                     check=False, capture_output=True, text=True, timeout=2, env=env)
        if result.returncode != 0:
            return None
        desktop_id = result.stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return None
    if not desktop_id or Path(desktop_id).name != desktop_id or not desktop_id.endswith(".desktop"):
        return None
    desktop_file = next((directory / desktop_id for directory in _data_dirs(env)
                         if (directory / desktop_id).is_file()), None)
    if desktop_file is None:
        return None
    try:
        lines = desktop_file.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return None
    in_entry = False
    command = None
    for line in lines:
        if line.startswith("[") and line.endswith("]"):
            in_entry = line == "[Desktop Entry]"
        elif in_entry and line.startswith("Exec="):
            command = line[5:]
            break
    if not command:
        return None
    try:
        argv = shlex.split(command, posix=True)
    except ValueError:
        return None
    if not argv or "%" in argv[0]:
        return None
    executable = argv[0]
    path = (_executable(executable) if "/" in executable
            else _executable(which(executable, path=env.get("PATH"))))
    if not path:
        return None
    family = _DEFAULT_EXECUTABLES.get(Path(executable).name)
    return path, family


def discover_browsers(explicit_path=None, *, env=None, which=shutil.which, run=subprocess.run):
    """Return executable candidates, preferring a supported system default.

    An explicit executable is validated and returned alone. Discovery reads no
    browser profiles and never launches a browser process.
    """
    environment = dict(os.environ if env is None else env)
    if explicit_path is not None:
        resolved = _executable(explicit_path)
        if resolved is None:
            raise ValueError("Browser path must name an executable file")
        return [BrowserCandidate("Selected browser", resolved)]

    found = []
    seen = set()
    for name, programs in _PROGRAMS:
        for program in programs:
            resolved = _executable(which(program, path=environment.get("PATH")))
            if resolved and resolved not in seen:
                found.append(BrowserCandidate(name, resolved))
                seen.add(resolved)
                break
    for name, paths in _KNOWN_PATHS:
        if any(item.name == name for item in found):
            continue
        for path in paths:
            resolved = _executable(path)
            if resolved and resolved not in seen:
                found.append(BrowserCandidate(name, resolved))
                seen.add(resolved)
                break

    default = _default_exec(environment, which=which, run=run)
    if default:
        default_path, default_family = default
        if default_family:
            for index, item in enumerate(found):
                if item.name == default_family:
                    found.insert(0, found.pop(index))
                    break
        for index, item in enumerate(found):
            if item.path == default_path:
                found.insert(0, found.pop(index))
                break
    return found
