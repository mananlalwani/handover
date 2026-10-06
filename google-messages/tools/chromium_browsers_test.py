import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import chromium_browsers as browsers


class BrowserDiscoveryTests(unittest.TestCase):
    def executable(self, path):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        path.chmod(0o755)
        return path

    def test_path_discovery_deduplicates_realpaths(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            bin_dir = root / "bin"
            chrome = self.executable(bin_dir / "google-chrome-stable")
            (bin_dir / "google-chrome").symlink_to(chrome)
            chromium = self.executable(bin_dir / "chromium")
            env = {"PATH": str(bin_dir), "HOME": str(root)}

            with patch.object(browsers, "_KNOWN_PATHS", ()):
                found = browsers.discover_browsers(
                    env=env,
                    run=lambda *args, **kwargs: type("Result", (), {"stdout": "", "returncode": 1})(),
                )

            self.assertEqual([(item.name, item.path) for item in found], [
                ("Google Chrome", str(chrome.resolve())),
                ("Chromium", str(chromium.resolve())),
            ])

    def test_supported_desktop_default_moves_first(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            bin_dir = root / "bin"
            chromium = self.executable(bin_dir / "chromium")
            brave = self.executable(bin_dir / "brave-browser")
            applications = root / "data/applications"
            applications.mkdir(parents=True)
            (applications / "brave-browser.desktop").write_text(
                "[Desktop Entry]\nName=Brave\nExec=brave-browser --some-option %U\n",
                encoding="utf-8",
            )
            env = {"PATH": str(bin_dir), "HOME": str(root), "XDG_DATA_HOME": str(root / "data")}
            result = type("Result", (), {"stdout": "brave-browser.desktop\n", "returncode": 0})()
            calls = []

            def run(args, **kwargs):
                calls.append(args)
                return result

            found = browsers.discover_browsers(env=env, run=run)

            self.assertEqual(found[0], browsers.BrowserCandidate("Brave", str(brave.resolve())))
            self.assertEqual(calls, [["xdg-settings", "get", "default-web-browser"]])
            self.assertEqual(found[1].path, str(chromium.resolve()))

    def test_helium_default_wrapper_selects_direct_browser_binary(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            bin_dir = root / "bin"
            helium = self.executable(bin_dir / "helium")
            self.executable(bin_dir / "helium-browser")
            chrome = self.executable(bin_dir / "google-chrome-stable")
            applications = root / "data/applications"
            applications.mkdir(parents=True)
            (applications / "helium.desktop").write_text(
                "[Desktop Entry]\nExec=helium-browser %U\n", encoding="utf-8"
            )
            env = {"PATH": str(bin_dir), "HOME": str(root), "XDG_DATA_HOME": str(root / "data")}
            result = type("Result", (), {"stdout": "helium.desktop", "returncode": 0})()

            with patch.object(browsers, "_KNOWN_PATHS", ()):
                found = browsers.discover_browsers(env=env, run=lambda *_a, **_k: result)

            self.assertEqual(found[0], browsers.BrowserCandidate("Helium", str(helium.resolve())))
            self.assertNotEqual(found[0].path, str((bin_dir / "helium-browser").resolve()))
            self.assertEqual(found[1].path, str(chrome.resolve()))

    def test_system_desktop_default_is_used_and_shell_text_is_not_run(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            bin_dir = root / "bin"
            chromium = self.executable(bin_dir / "chromium")
            marker = root / "unexpected"
            system_apps = root / "system/applications"
            system_apps.mkdir(parents=True)
            (system_apps / "chromium.desktop").write_text(
                f"[Desktop Entry]\nExec=chromium; touch {marker}\n", encoding="utf-8"
            )
            env = {"PATH": str(bin_dir), "HOME": str(root),
                   "XDG_DATA_HOME": str(root / "user"), "XDG_DATA_DIRS": str(root / "system")}
            result = type("Result", (), {"stdout": "chromium.desktop", "returncode": 0})()
            found = browsers.discover_browsers(env=env, run=lambda *_a, **_k: result)
            self.assertEqual(found[0].path, str(chromium.resolve()))
            self.assertFalse(marker.exists())

    def test_non_chromium_default_does_not_change_inventory_order(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            bin_dir = root / "bin"
            chrome = self.executable(bin_dir / "google-chrome")
            other = self.executable(bin_dir / "firefox")
            apps = root / "data/applications"
            apps.mkdir(parents=True)
            (apps / "firefox.desktop").write_text(
                "[Desktop Entry]\nExec=firefox %u\n", encoding="utf-8"
            )
            env = {"PATH": str(bin_dir), "HOME": str(root), "XDG_DATA_HOME": str(root / "data")}
            result = type("Result", (), {"stdout": "firefox.desktop", "returncode": 0})()
            found = browsers.discover_browsers(env=env, run=lambda *_a, **_k: result)
            self.assertEqual(found[0].path, str(chrome.resolve()))
            self.assertNotIn(str(other.resolve()), [item.path for item in found])

    def test_explicit_path_must_be_executable_and_is_not_launched(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            browser = self.executable(root / "custom-browser")
            selected = browsers.discover_browsers(str(browser))
            self.assertEqual(selected, [browsers.BrowserCandidate("Selected browser", str(browser.resolve()))])
            non_executable = root / "text-file"
            non_executable.write_text("anything", encoding="utf-8")
            with self.assertRaises(ValueError):
                browsers.discover_browsers(str(non_executable))
            with self.assertRaises(ValueError):
                browsers.discover_browsers(str(root / "missing"))


if __name__ == "__main__":
    unittest.main()
