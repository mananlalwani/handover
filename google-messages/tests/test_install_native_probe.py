import hashlib
import importlib.util
import json
import os
import shlex
import stat
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


MODULE_PATH = Path(__file__).parents[1] / "tools" / "install_native_probe.py"
SPEC = importlib.util.spec_from_file_location("install_native_probe", MODULE_PATH)
installer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(installer)


class InstallNativeProbeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.home = self.root / "home"
        self.home.mkdir()
        self.extension_dir = self.root / "extension dir"
        self.extension_dir.mkdir()
        self.binary = self.root / "auth probe"
        self.binary.write_text("#!/bin/sh\nexit 0\n")
        self.binary.chmod(0o755)
        self.environment = patch.dict(os.environ, {"HOME": str(self.home), "XDG_CONFIG_HOME": str(self.root / "config")})
        self.environment.start()

    def tearDown(self):
        self.environment.stop()
        self.temp.cleanup()

    def test_computes_chromium_id_and_installs_private_atomic_files(self):
        expected = "".join(
            chr(ord("a") + nibble)
            for byte in hashlib.sha256(str(self.extension_dir.resolve()).encode("utf-8")).digest()[:16]
            for nibble in (byte >> 4, byte & 0x0F)
        )
        extension_id, launcher, manifest_path = installer.install(self.binary, self.extension_dir)
        self.assertEqual(extension_id, expected)
        manifest = json.loads(manifest_path.read_text())
        self.assertEqual(manifest["allowed_origins"], [f"chrome-extension://{expected}/"])
        self.assertEqual(manifest["path"], str(launcher))
        self.assertEqual(launcher.stat().st_mode & 0o777, 0o700)
        self.assertEqual(manifest_path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(launcher.parent.stat().st_mode & 0o777, 0o700)
        self.assertEqual(manifest_path.parent.stat().st_mode & 0o777, 0o700)
        wrapper = launcher.read_text()
        self.assertIn(shlex.quote(str(self.binary)), wrapper)
        self.assertIn('"$@"', wrapper)
        self.assertNotIn("cookie", wrapper.lower())

        installer.install(self.binary, self.extension_dir)
        self.assertTrue(launcher.exists())

    def test_validates_override_and_rejects_symlink_or_nonexecutable_binary(self):
        for invalid_id in ("bad", ""):
            with self.assertRaisesRegex(installer.InstallError, "invalid_extension_id"):
                installer.install(self.binary, self.extension_dir, invalid_id)
        link = self.root / "linked-binary"
        link.symlink_to(self.binary)
        with self.assertRaisesRegex(installer.InstallError, "invalid_binary"):
            installer.install(link, self.extension_dir)
        plain = self.root / "plain-file"
        plain.write_text("not executable")
        with self.assertRaisesRegex(installer.InstallError, "invalid_binary"):
            installer.install(plain, self.extension_dir)

    def test_does_not_replace_unrelated_native_host_manifest(self):
        host_dir = self.root / "config" / "google-chrome" / "NativeMessagingHosts"
        host_dir.mkdir(parents=True)
        target = host_dir / f"{installer.HOST_NAME}.json"
        target.write_text(json.dumps({"name": "some.other.host", "path": "/bin/true"}))
        with self.assertRaisesRegex(installer.InstallError, "unrelated_existing_file"):
            installer.install(self.binary, self.extension_dir)
        self.assertEqual(json.loads(target.read_text())["name"], "some.other.host")
        self.assertFalse((self.home / ".local/lib/handover/google-messages-auth-probe/launch-auth-probe").exists())

    def test_extension_id_override_is_used_as_the_only_allowed_origin(self):
        override = "p" * 32
        extension_id, _, manifest_path = installer.install(self.binary, self.extension_dir, override)
        self.assertEqual(extension_id, override)
        self.assertEqual(json.loads(manifest_path.read_text())["allowed_origins"], [
            f"chrome-extension://{override}/"
        ])


if __name__ == "__main__":
    unittest.main()
