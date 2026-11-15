"""Resolve Rust test executables from a Cargo JSON artifact stream."""

import json
import os
from pathlib import Path
import subprocess


def _test_executable_from_json(output: str, target_name: str, source: str, root: Path) -> Path:
    matches = set()
    for line_number, line in enumerate(output.splitlines(), 1):
        if not line.strip():
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError as error:
            raise SystemExit(f"Invalid Cargo JSON in {source} at line {line_number}: {error}") from error
        if not isinstance(message, dict):
            raise SystemExit(f"Invalid Cargo JSON in {source} at line {line_number}: expected an object")
        target = message.get("target")
        profile = message.get("profile")
        executable = message.get("executable")
        if (message.get("reason") == "compiler-artifact"
                and isinstance(target, dict) and target.get("name") == target_name
                and isinstance(profile, dict) and profile.get("test") is True
                and isinstance(executable, str) and executable):
            path = Path(executable)
            if not path.is_absolute():
                path = root / path
            matches.add(path.resolve())

    if len(matches) != 1:
        raise SystemExit(
            f"Expected exactly one test executable for Cargo target {target_name!r} in {source}; "
            f"found {len(matches)}"
        )
    executable = matches.pop()
    if not executable.is_file() or not os.access(executable, os.X_OK):
        raise SystemExit(f"Cargo test executable for target {target_name!r} is missing or not executable: {executable}")
    return executable


def test_executable(target_name: str, build_command: list[str], root: Path) -> Path:
    """Use HANDOVER_TEST_ARTIFACTS when set, otherwise run the targeted Cargo build."""
    manifest = os.environ.get("HANDOVER_TEST_ARTIFACTS")
    if manifest is not None:
        if not manifest:
            raise SystemExit("HANDOVER_TEST_ARTIFACTS is set but empty")
        manifest_path = Path(manifest)
        try:
            output = manifest_path.read_text(encoding="utf-8")
        except OSError as error:
            raise SystemExit(f"Cannot read HANDOVER_TEST_ARTIFACTS manifest {manifest_path}: {error}") from error
        return _test_executable_from_json(output, target_name, str(manifest_path), root)

    built = subprocess.run(
        build_command,
        cwd=root,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    )
    return _test_executable_from_json(built.stdout, target_name, "targeted Cargo build output", root)
