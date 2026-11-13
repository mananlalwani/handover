#!/usr/bin/env python3
"""Exercise app discovery, user installation, and startup with isolated homes."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

HELPER = Path(__file__).resolve().parent.parent / "quickshell/MessagesApp.py"


class MessagesAppTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="handover-app-")
        self.root = Path(self.temporary.name)
        self.home = self.root / "home with spaces"
        self.home.mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.env = dict(os.environ, HOME=str(self.home), PATH=str(self.bin),
                        XDG_DATA_HOME=str(self.root / "data"))
        self.env.pop("HANDOVER_BIN_DIR", None)

    def tearDown(self):
        self.temporary.cleanup()

    def run_helper(self, *arguments):
        result = subprocess.run([sys.executable, str(HELPER), *map(str, arguments)],
                                env=self.env, capture_output=True, text=True, timeout=5)
        payload = json.loads(result.stdout)
        self.assertEqual(result.returncode == 0, payload["success"])
        return payload

    def test_missing_app_remains_available_for_fallback(self):
        self.assertEqual(self.run_helper("status"), {"success": True, "installed": False})
        result = self.run_helper("launch")
        self.assertFalse(result["success"])
        self.assertFalse(result["installed"])

    def test_finds_app_beside_launcher_in_custom_installation(self):
        custom_bin = self.root / "custom prefix/bin"
        custom_bin.mkdir(parents=True)
        binary = custom_bin / "handover-messages"
        shutil.copyfile("/usr/bin/true", binary)
        binary.chmod(0o755)
        self.env["HANDOVER_BIN_DIR"] = str(custom_bin)
        self.assertTrue(self.run_helper("status")["installed"])
        self.assertTrue(self.run_helper("launch")["success"])

    def test_detects_path_app_and_reports_startup_failure(self):
        shutil.copyfile("/usr/bin/false", self.bin / "handover-messages")
        (self.bin / "handover-messages").chmod(0o755)
        self.assertTrue(self.run_helper("status")["installed"])
        self.assertFalse(self.run_helper("launch")["success"])

    def test_installs_only_app_and_desktop_entry_without_running_source(self):
        source = self.root / 'selected $(touch should-not-exist)'
        shutil.copyfile("/usr/bin/true", source)
        result = self.run_helper("install", source)
        self.assertTrue(result["success"])
        destination = self.home / ".local/bin/handover-messages"
        self.assertEqual(destination.read_bytes(), source.read_bytes())
        self.assertEqual(destination.stat().st_mode & 0o777, 0o755)
        desktop = self.root / "data/applications/handover-messages.desktop"
        self.assertIn(f'TryExec={destination}', desktop.read_text())
        self.assertFalse((self.home / ".local/bin/handoverd").exists())
        self.assertFalse((self.root / "data/systemd").exists())
        self.assertFalse((self.root / "should-not-exist").exists())
        self.assertTrue(self.run_helper("status")["installed"])
        self.assertTrue(self.run_helper("launch")["success"])

    def test_invalid_selection_does_not_replace_existing_app(self):
        self.run_helper("install", "/usr/bin/true")
        destination = self.home / ".local/bin/handover-messages"
        before = destination.read_bytes()
        source = self.root / "archive.tar.gz"
        source.write_bytes(b"not an ELF executable")
        result = self.run_helper("install", source)
        self.assertFalse(result["success"])
        self.assertEqual(destination.read_bytes(), before)

    @unittest.skipUnless(shutil.which("gio"), "GIO desktop launcher is unavailable")
    def test_desktop_launcher_handles_reserved_path_characters(self):
        gio = shutil.which("gio")
        self.home = self.root / 'home with spaces % $ ` " \\ characters'
        self.home.mkdir()
        self.env["HOME"] = str(self.home)
        self.run_helper("install", "/usr/bin/true")
        marker = self.root / "desktop-launched"
        binary = self.home / ".local/bin/handover-messages"
        binary.write_text(f'#!/bin/sh\nprintf done > "{marker}"\n')
        desktop = self.root / "data/applications/handover-messages.desktop"
        subprocess.run([gio, "launch", str(desktop)], env=self.env,
                       check=True, capture_output=True, timeout=5)
        for _ in range(30):
            if marker.exists():
                break
            time.sleep(0.05)
        self.assertEqual(marker.read_text(), "done")

    def test_launched_process_survives_helper_exit(self):
        script = self.bin / "handover-messages"
        marker = self.root / "finished"
        script.write_text(f'#!/bin/sh\n/bin/sleep 0.8\nprintf done > "{marker}"\n')
        script.chmod(0o755)
        self.assertTrue(self.run_helper("launch")["success"])
        # The child writes after the helper exits; closing QML must not own it.
        for _ in range(30):
            if marker.exists():
                break
            time.sleep(0.05)
        self.assertEqual(marker.read_text(), "done")


if __name__ == "__main__":
    unittest.main()
