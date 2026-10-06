import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import setup_google_messages as setup
from chromium_browsers import BrowserCandidate


class SetupTests(unittest.TestCase):
    def test_pairing_symbol_survives_native_status_forwarding(self):
        events = [
            {"status": "progress", "message": "Confirm on phone", "verification": "🧺"},
            {"status": "progress", "message": "Waiting", "verification": None},
            {"status": "saved", "message": "Saved", "account": "opaque-test"},
        ]
        process = Mock()
        process.stdout = io.BytesIO(b"".join(
            json.dumps(event).encode() + b"\n" for event in events))
        process.wait.return_value = 0
        process.poll.return_value = 0
        output = io.StringIO()
        with patch.object(setup.subprocess, "Popen", return_value=process), \
             contextlib.redirect_stdout(output):
            setup.native_setup("synthetic-probe", {})
        forwarded = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(forwarded[0]["verification"], "🧺")
        self.assertNotIn("verification", forwarded[1])
        self.assertEqual(forwarded[2]["status"], "saved")

    def test_pause_is_released_when_setup_raises(self):
        requests = []

        def request(method, **fields):
            requests.append((method, fields))
            if method == "messages.setup.begin":
                return {"type": "messaging_setup_paused", "lease": "owned"}
            return {"type": "messaging_setup_resumed"}

        with patch.object(setup, "daemon_request", request):
            with self.assertRaises(RuntimeError):
                with setup.MessagingPause():
                    raise RuntimeError("synthetic failure")
        self.assertEqual(requests, [("messages.setup.begin", {}),
                                    ("messages.setup.end", {"lease": "owned"})])

    def test_setup_order_and_proof_cleanup_on_cancellation(self):
        events = []
        proof = {"authorization": "synthetic-private"}

        class Pause:
            def __enter__(self):
                events.append("pause")

            def __exit__(self, *_):
                events.append("resume")

        def browser(*_, **__):
            events.append("owned-browser-closed")
            return proof

        def native(*_):
            events.append("native")
            self.assertEqual(events, ["pause", "owned-browser-closed", "native"])
            raise KeyboardInterrupt()

        with tempfile.TemporaryDirectory() as temp:
            executable = Path(temp) / "native"
            executable.write_text("#!/bin/sh\nexit 0\n")
            executable.chmod(0o755)
            output = io.StringIO()
            with patch.object(setup, "discover_browsers", return_value=[BrowserCandidate("Test", "/synthetic/browser")]), \
                 patch.object(setup, "MessagingPause", Pause), \
                 patch.object(setup, "browser_sign_in", browser), \
                 patch.object(setup, "native_setup", native), \
                 patch("sys.argv", ["setup", "--native-probe", str(executable)]), \
                 patch.object(setup.signal, "signal"), contextlib.redirect_stdout(output):
                self.assertEqual(setup.main(), 130)
        self.assertEqual(events[-1], "resume")
        self.assertEqual(proof, {})
        self.assertNotIn("synthetic-private", output.getvalue())


if __name__ == "__main__":
    unittest.main()
