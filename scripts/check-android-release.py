#!/usr/bin/env python3
"""Reject a release tag whose Android package cannot update the prior release."""

import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ANDROID_BUILD = ROOT / "android/app/build.gradle.kts"
VERSION = re.compile(r"v([0-9]+)\.([0-9]+)\.([0-9]+)")


def android_version(contents: str) -> tuple[str, int]:
    name = re.search(r'\bversionName\s*=\s*"([^"]+)"', contents)
    code = re.search(r"\bversionCode\s*=\s*([0-9]+)", contents)
    if not name or not code:
        raise ValueError("Android versionName or versionCode is missing")
    return name.group(1), int(code.group(1))


def main(tag: str) -> None:
    parsed = VERSION.fullmatch(tag)
    if parsed is None:
        raise ValueError(f"expected a vX.Y.Z tag, got {tag!r}")
    version = tag[1:]
    cargo = tomllib.loads((ROOT / "handoverd/Cargo.toml").read_text())["package"]["version"]
    android_name, android_code = android_version(ANDROID_BUILD.read_text())
    if cargo != version or android_name != version:
        raise ValueError(
            f"tag {tag}, handoverd {cargo}, and Android {android_name} must match"
        )

    tags = subprocess.check_output(["git", "tag", "--list", "v*"], cwd=ROOT, text=True)
    older = [
        (tuple(map(int, match.groups())), candidate)
        for candidate in tags.splitlines()
        if (match := VERSION.fullmatch(candidate))
        and tuple(map(int, match.groups())) < tuple(map(int, parsed.groups()))
    ]
    if older:
        previous = max(older)[1]
        previous_build = subprocess.check_output(
            ["git", "show", f"{previous}:android/app/build.gradle.kts"],
            cwd=ROOT,
            text=True,
        )
        _, previous_code = android_version(previous_build)
        if android_code <= previous_code:
            raise ValueError(
                f"Android versionCode {android_code} must exceed {previous_code} from {previous}"
            )
    print(f"Android release {tag}: versionCode {android_code} checked")


if __name__ == "__main__":
    try:
        main(sys.argv[1])
    except (IndexError, KeyError, ValueError, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error)) from error
