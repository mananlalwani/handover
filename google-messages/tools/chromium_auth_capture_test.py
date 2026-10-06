import contextlib
import io
import json
import struct
import unittest
from unittest.mock import patch

import chromium_auth_capture as auth


class CaptureTests(unittest.TestCase):
    def test_request_allowlist(self):
        for url in ["http://instantmessaging-pa.googleapis.com" + auth.SIGN_IN_PATH,
                    "https://evil.test" + auth.SIGN_IN_PATH,
                    "https://instantmessaging-pa.googleapis.com:444" + auth.SIGN_IN_PATH,
                    "https://person@instantmessaging-pa.googleapis.com" + auth.SIGN_IN_PATH,
                    "https://instantmessaging-pa.googleapis.com/other"]:
            self.assertIsNone(auth.endpoint_for({"url": url, "method": "POST"}))
        self.assertIsNone(auth.endpoint_for({"url": "https://instantmessaging-pa.googleapis.com"
                                            + auth.SIGN_IN_PATH, "method": "GET"}))

    def test_header_bounds_and_early_cookie_exclusion(self):
        self.assertEqual(auth.selected_headers({"Cookie": "SID=synthetic"}, False), {})
        for value in ["a\r\nb", "x" * 8193]:
            with self.assertRaises(RuntimeError):
                auth.selected_headers({"Authorization": value}, True)

    def test_single_matched_proof_goes_only_to_local_stdin(self):
        events = iter([
            {"sessionId": "s", "method": "Network.requestWillBeSentExtraInfo",
             "params": {"requestId": "unmatched", "headers": {"Cookie": "private-unmatched"}}},
            {"sessionId": "s", "method": "Network.requestWillBeSent", "params": {
                "requestId": "matched", "request": {"method": "POST",
                    "url": "https://instantmessaging-pa.googleapis.com" + auth.SIGN_IN_PATH,
                    "headers": {"Authorization": "Bearer synthetic", "X-Goog-Api-Key": "synthetic"}}}},
            {"sessionId": "s", "method": "Network.requestWillBeSentExtraInfo", "params": {
                "requestId": "matched", "headers": {"Cookie": "SID=synthetic",
                                                        "Origin": "https://messages.google.com"}}},
        ])

        class FakePipe:
            def __init__(self, *_):
                self.pending = b""

            def call(self, method, *_):
                if method == "Target.getTargets":
                    return {"targetInfos": [{"type": "page", "url": "about:blank", "targetId": "t"}]}
                if method == "Target.attachToTarget":
                    return {"sessionId": "s"}
                return {}

            def send(self, *_):
                pass

            def read(self):
                return next(events)

        reply = json.dumps({"ok": True, "sources": 4}).encode()
        result = type("Result", (), {"returncode": 0, "stdout": struct.pack("=I", len(reply)) + reply})()
        output = io.StringIO()
        with patch.object(auth, "Pipe", FakePipe), patch.object(auth.subprocess, "run", return_value=result) as run:
            with contextlib.redirect_stdout(output):
                auth.capture(0, 1, "/synthetic/probe")
        run.assert_called_once()
        self.assertEqual(run.call_args.args[0], ["/synthetic/probe", "--local-read-only"])
        data = run.call_args.kwargs["input"]
        self.assertEqual(struct.unpack("=I", data[:4])[0], len(data) - 4)
        proof = json.loads(data[4:])
        self.assertEqual(proof["type"], "gaia_lookup_with_cookies")
        self.assertEqual(proof["service_cookie"], "SID=synthetic")
        self.assertNotIn(b"private-unmatched", data)
        self.assertNotIn("synthetic", output.getvalue())


if __name__ == "__main__":
    unittest.main()
