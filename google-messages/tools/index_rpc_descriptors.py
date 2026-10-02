#!/usr/bin/env python3
"""Index observed RPC descriptor literals from a verified bootstrap capture."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import stat
import sys
from pathlib import Path, PurePosixPath


MANIFEST_LIMIT = 1024 * 1024
SCRIPT_LIMIT = 8 * 1024 * 1024
SCRIPT_COUNT_LIMIT = 8
TOTAL_LIMIT = 32 * 1024 * 1024
IDENTIFIER = r"(?:[A-Za-z_$][A-Za-z0-9_$]*|_\.[A-Za-z_$][A-Za-z0-9_$]*)"
RPC_PATH = r"/?google\.internal\.communications\.instantmessaging\.v1\.[A-Za-z0-9_.]+/[A-Za-z0-9]+"
CANDIDATE_RPC_PATH = r"/?google\.internal\.[A-Za-z0-9_.]+/[A-Za-z0-9]+"
CANDIDATE_RE = re.compile(rf"(?<![A-Za-z0-9_.]){CANDIDATE_RPC_PATH}(?![A-Za-z0-9_])")
DESCRIPTOR_RE = re.compile(
    rf"new\s+_\.jH\(\s*\"(?P<path>/{RPC_PATH.lstrip('/')})\"\s*,\s*"
    rf"(?P<request>{IDENTIFIER})\s*,\s*(?P<response>{IDENTIFIER})\s*,"
)


class IndexError(Exception):
    """The capture is malformed, unsafe, or exceeds an indexing bound."""


def _read_bounded_regular(path: Path, limit: int, label: str) -> bytes:
    flags = os.O_RDONLY | getattr(os, "O_NONBLOCK", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(path, flags)
    except OSError as exc:
        raise IndexError(f"cannot open {label}: {exc}") from exc
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise IndexError(f"{label} must be a regular file")
        with os.fdopen(fd, "rb") as stream:
            fd = -1
            data = stream.read(limit + 1)
    finally:
        if fd >= 0:
            os.close(fd)
    if len(data) > limit:
        raise IndexError(f"{label} exceeds {limit} byte limit")
    return data


def _read_manifest(capture_dir: Path) -> dict:
    path = capture_dir / "manifest.json"
    if path.is_symlink():
        raise IndexError("manifest.json must not be a symlink")
    data = _read_bounded_regular(path, MANIFEST_LIMIT, "manifest.json")
    try:
        manifest = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise IndexError(f"manifest is not valid JSON: {exc}") from exc
    if not isinstance(manifest, dict):
        raise IndexError("manifest must be a JSON object")
    scripts = manifest.get("scripts")
    if not isinstance(scripts, list):
        raise IndexError("manifest scripts must be a list")
    if len(scripts) > SCRIPT_COUNT_LIMIT:
        raise IndexError(f"manifest lists more than {SCRIPT_COUNT_LIMIT} scripts")
    return manifest


def _script_path(capture_dir: Path, filename: object) -> Path:
    if not isinstance(filename, str) or not filename or "\\" in filename:
        raise IndexError("script file must be a nonempty relative path")
    relative = PurePosixPath(filename)
    if relative.is_absolute() or any(part in ("", ".", "..") for part in relative.parts):
        raise IndexError(f"unsafe script path: {filename!r}")
    path = capture_dir.joinpath(*relative.parts)
    current = capture_dir
    for part in relative.parts:
        current = current / part
        if current.is_symlink():
            raise IndexError(f"script path contains a symlink: {filename!r}")
    try:
        path.resolve(strict=True).relative_to(capture_dir.resolve(strict=True))
    except (OSError, ValueError) as exc:
        raise IndexError(f"script path escapes capture directory or is missing: {filename!r}") from exc
    return path


def index_capture(capture_dir: Path) -> dict:
    capture_dir = Path(capture_dir)
    manifest = _read_manifest(capture_dir)
    records = []
    candidate_count = 0
    total = 0
    for entry in manifest["scripts"]:
        if not isinstance(entry, dict):
            raise IndexError("each script entry must be an object")
        path = _script_path(capture_dir, entry.get("file"))
        data = _read_bounded_regular(path, SCRIPT_LIMIT, f"script {entry['file']!r}")
        expected_size = entry.get("size")
        if isinstance(expected_size, bool) or not isinstance(expected_size, int) or expected_size != len(data):
            raise IndexError(f"script size mismatch: {entry['file']}")
        expected_hash = entry.get("sha256")
        actual_hash = hashlib.sha256(data).hexdigest()
        if not isinstance(expected_hash, str) or expected_hash != actual_hash:
            raise IndexError(f"script SHA-256 mismatch: {entry['file']}")
        total += len(data)
        if total > TOTAL_LIMIT:
            raise IndexError(f"scripts exceed {TOTAL_LIMIT} byte total limit")
        try:
            source = data.decode("utf-8")
        except UnicodeDecodeError as exc:
            raise IndexError(f"script is not valid UTF-8: {entry['file']}") from exc
        candidate_count += sum(1 for _ in CANDIDATE_RE.finditer(source))
        for match in DESCRIPTOR_RE.finditer(source):
            records.append({
                "rpc_path": match.group("path"),
                "request_symbol": match.group("request"),
                "response_symbol": match.group("response"),
                "script_file": entry["file"],
                "script_sha256": actual_hash,
                "source_character_offset": match.start("path"),
            })
    return {
        "candidate_count": candidate_count,
        "indexed_count": len(records),
        "complete": False,
        "records": records,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture_directory", type=Path)
    args = parser.parse_args(argv)
    try:
        result = index_capture(args.capture_directory)
    except (IndexError, OSError) as exc:
        print(f"index_rpc_descriptors: {exc}", file=sys.stderr)
        return 1
    json.dump(result, sys.stdout, indent=2)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
