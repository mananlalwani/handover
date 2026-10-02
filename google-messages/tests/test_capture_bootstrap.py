import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


MODULE_PATH = Path(__file__).parents[1] / "tools" / "capture_bootstrap.py"
SPEC = importlib.util.spec_from_file_location("capture_bootstrap", MODULE_PATH)
capture_bootstrap = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(capture_bootstrap)


class CaptureBootstrapTests(unittest.TestCase):
    def test_validates_page_and_script_origins_and_redirects_before_following(self):
        with self.assertRaises(capture_bootstrap.CaptureError):
            capture_bootstrap.validate_url("https://evil.example/_/messagesweb/a.js", "script")
        with self.assertRaises(capture_bootstrap.CaptureError):
            capture_bootstrap.validate_url("https://user@www.gstatic.com/_/messagesweb/a.js", "script")
        with self.assertRaises(capture_bootstrap.CaptureError):
            capture_bootstrap.validate_url("https://messages.google.com.evil/web/", "page")

        handler = capture_bootstrap._SafeRedirectHandler("script")
        with self.assertRaises(capture_bootstrap.CaptureError):
            handler.redirect_request(None, None, 302, "Found", {}, "https://evil.example/x.js")

    def test_resolves_relative_script_urls_and_extracts_rpc_candidates(self):
        page = (b'<html><base href="https://www.gstatic.com/_/messagesweb/">'
                b'<script src="app.js"></script></html>')
        js = b'"google.internal.foo.Bar/GetThing"; "google.internal.foo.Bar/GetThing"; "google.internal.X/Do"'
        responses = {
            capture_bootstrap.PAGE_URL: page,
            "https://www.gstatic.com/_/messagesweb/app.js": js,
        }
        calls = []

        def fake_fetch(url, kind, limit):
            calls.append((url, kind, limit))
            return responses[url]

        with tempfile.TemporaryDirectory() as parent:
            output = Path(parent) / "private"
            with patch.object(capture_bootstrap, "_fetch", side_effect=fake_fetch):
                total, script_count, method_count = capture_bootstrap.capture(output)
            manifest = json.loads((output / "manifest.json").read_text())
            self.assertEqual(calls[1][0], "https://www.gstatic.com/_/messagesweb/app.js")
            self.assertEqual(script_count, 1)
            self.assertEqual(method_count, 2)
            self.assertEqual(total, len(page) + len(js))
            self.assertEqual(manifest["candidate_rpc_methods"], [
                "google.internal.X/Do", "google.internal.foo.Bar/GetThing"
            ])
            self.assertEqual(manifest["scripts"][0]["source_url"], calls[1][0])
            self.assertEqual(output.stat().st_mode & 0o777, 0o700)
            self.assertEqual((output / "page.html").stat().st_mode & 0o777, 0o600)
            self.assertEqual((output / "manifest.json").stat().st_mode & 0o777, 0o600)

    def test_rejects_malicious_asset_before_fetch_and_cleans_partial_capture(self):
        page = b'<script src="https://attacker.example/steal.js"></script>'
        calls = []
        with tempfile.TemporaryDirectory() as parent:
            output = Path(parent) / "capture"
            with patch.object(capture_bootstrap, "_fetch", side_effect=lambda *args: calls.append(args) or page):
                with self.assertRaises(capture_bootstrap.CaptureError):
                    capture_bootstrap.capture(output)
            self.assertEqual(len(calls), 1)
            self.assertFalse(output.exists())

    def test_enforces_page_script_count_total_and_existing_directory_bounds(self):
        with tempfile.TemporaryDirectory() as parent:
            existing = Path(parent) / "existing"
            existing.mkdir()
            with patch.object(capture_bootstrap, "_fetch") as fetch:
                with self.assertRaises(capture_bootstrap.CaptureError):
                    capture_bootstrap.capture(existing)
                fetch.assert_not_called()

            output = Path(parent) / "too-many"
            refs = "".join(
                f'<script src="https://www.gstatic.com/_/messagesweb/{i}.js"></script>'
                for i in range(capture_bootstrap.SCRIPT_COUNT_LIMIT + 1)
            ).encode()
            with patch.object(capture_bootstrap, "_fetch", return_value=refs):
                with self.assertRaises(capture_bootstrap.CaptureError):
                    capture_bootstrap.capture(output)
            self.assertFalse(output.exists())

            output = Path(parent) / "too-large"
            page = b'<script src="https://www.gstatic.com/_/messagesweb/a.js"></script>'

            def too_large(url, kind, limit):
                return page if kind == "page" else b"x" * (capture_bootstrap.SCRIPT_LIMIT + 1)

            with patch.object(capture_bootstrap, "_fetch", side_effect=too_large):
                with self.assertRaises(capture_bootstrap.CaptureError):
                    capture_bootstrap.capture(output)
            self.assertFalse(output.exists())

    def test_total_limit_is_enforced_when_each_script_is_within_its_limit(self):
        page = (b'<script src="https://www.gstatic.com/_/messagesweb/a.js"></script>'
                b'<script src="https://www.gstatic.com/_/messagesweb/b.js"></script>')
        calls = []
        def fetch(url, kind, limit):
            calls.append(kind)
            return page if kind == "page" else b"x" * 8
        with tempfile.TemporaryDirectory() as parent:
            output = Path(parent) / "too-large-total"
            with patch.object(capture_bootstrap, "SCRIPT_LIMIT", 8), \
                 patch.object(capture_bootstrap, "TOTAL_LIMIT", len(page) + 15), \
                 patch.object(capture_bootstrap, "_fetch", side_effect=fetch):
                with self.assertRaisesRegex(capture_bootstrap.CaptureError, "total limit"):
                    capture_bootstrap.capture(output)
            self.assertEqual(calls, ["page", "script", "script"])
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
