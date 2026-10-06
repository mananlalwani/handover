import contextlib
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import setup_google_messages as setup
from chromium_browsers import BrowserCandidate


class SetupTests(unittest.TestCase):
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
