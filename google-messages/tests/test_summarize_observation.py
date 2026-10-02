import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).parents[1] / "tools" / "summarize_observation.py"
SPEC = importlib.util.spec_from_file_location("summarize_observation", MODULE_PATH)
summarizer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(summarizer)


def record(**updates):
    value = {"type": "request", "service": "Messaging", "method": "PullMessages"}
    value.update(updates)
    return value


class SummarizeObservationTests(unittest.TestCase):
    def test_summarizes_only_allowlisted_aggregate_facts(self):
        records = [
            record(httpMethod="POST", contentType="application/json; charset=utf-8", body={
                "type": "array", "length": 4,
                "items": [{"type": "string", "bytes": 31}, {"type": "null"}],
            }),
            record(type="response", status=200, contentType="application/json", body={
                "type": "object", "propertyCount": 1,
                "values": [{"type": "boolean"}],
            }),
            record(type="response", status=200, body={"type": "protobuf_wire", "bytes": 23, "fields": [
                {"field": 1, "wireType": 0}, {"field": 2, "wireType": 2, "bytes": 18},
            ]}),
        ]
        result = summarizer.summarize(records)
        self.assertEqual(result["record_count"], 3)
        facts = result["rpcs"][0]
        self.assertEqual(facts["rpc"], "Messaging/PullMessages")
        self.assertEqual(facts["count"], 3)
        self.assertEqual(facts["requests"], 1)
        self.assertEqual(facts["statuses"], {"200": 2})
        self.assertEqual(facts["content_types"], {"application/json": 2})
        self.assertEqual(facts["body_types"], {"array": 1, "object": 1, "protobuf_wire": 1})
        self.assertEqual(facts["top_level_array_lengths"], {"4": 1})
        output = json.dumps(result)
        self.assertNotIn("items", output)
        self.assertNotIn("propertyCount", output)
        self.assertNotIn("wireType", output)

    def test_rejects_unknown_keys_and_string_payloads_without_echoing_values(self):
        for bad in (
            record(payload="BODY_SECRET_VALUE"),
            record(body={"type": "string", "bytes": 4, "value": "BODY_SECRET_VALUE"}),
            record(body={"type": "object", "propertyCount": 1,
                         "keys": ["secretKey"], "values": [{"type": "string", "bytes": 9}]}),
        ):
            with self.assertRaises(summarizer.ObservationError):
                summarizer.summarize([bad])

    def test_rejects_depth_node_count_and_bad_wire_types(self):
        body = {"type": "null"}
        for _ in range(summarizer.DEPTH_LIMIT + 1):
            body = {"type": "array", "length": 1, "items": [body]}
        with self.assertRaisesRegex(summarizer.ObservationError, "body_depth_limit"):
            summarizer.summarize([record(body=body)])

        many = {"type": "array", "length": 128, "items": [
            {"type": "object", "propertyCount": 512,
             "values": [{"type": "null"} for _ in range(512)]}
            for _ in range(128)
        ]}
        with self.assertRaisesRegex(summarizer.ObservationError, "body_node_limit"):
            summarizer.summarize([record(body=many) for _ in range(5)])

        for field in (
            {"field": 1, "wireType": 3},
            {"field": 1, "wireType": True},
            {"field": 0, "wireType": 0},
            {"field": 536870912, "wireType": 0},
            {"field": 2, "wireType": 2},
            {"field": 1, "wireType": 0, "bytes": 4},
        ):
            with self.assertRaises(summarizer.ObservationError):
                summarizer.summarize([record(body={"type": "protobuf_wire", "bytes": 23, "fields": [
                    field
                ]})])

    def test_rejects_bad_rpc_status_record_count_and_file_bounds(self):
        for bad in (
            record(service="Unknown"), record(status=True), record(status=600),
            record(type=[]), record(httpMethod=[]), record(contentType=["application/json"]),
        ):
            with self.assertRaises(summarizer.ObservationError):
                summarizer.summarize([bad])
        with self.assertRaisesRegex(summarizer.ObservationError, "record_limit"):
            summarizer.summarize([record()] * (summarizer.RECORD_LIMIT + 1))

        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            large = root / "large.json"
            large.write_bytes(b" " * (summarizer.FILE_LIMIT + 1))
            with self.assertRaisesRegex(summarizer.ObservationError, "input_too_large"):
                summarizer.summarize_file(large)
            link = root / "link.json"
            link.symlink_to(large)
            with self.assertRaisesRegex(summarizer.ObservationError, "input_file_unavailable"):
                summarizer.summarize_file(link)

    def test_rejects_nonregular_files_and_malformed_json(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            malformed = root / "malformed.json"
            malformed.write_text("not-json")
            with self.assertRaisesRegex(summarizer.ObservationError, "invalid_json"):
                summarizer.summarize_file(malformed)
            if hasattr(os, "mkfifo"):
                fifo = root / "pipe.json"
                os.mkfifo(fifo)
                with self.assertRaisesRegex(summarizer.ObservationError, "input_not_regular_file"):
                    summarizer.summarize_file(fifo)

    def test_handles_deep_json_with_fixed_error_and_accepts_truncated_field_counts(self):
        with tempfile.TemporaryDirectory() as temp:
            deep = Path(temp) / "deep.json"
            deep.write_text("[" * 10000 + "]" * 10000)
            with self.assertRaisesRegex(summarizer.ObservationError, "invalid_record"):
                summarizer.summarize_file(deep)

        result = summarizer.summarize([record(body={
            "type": "object", "propertyCount": 700,
            "values": [{"type": "null"}] * 512, "truncated": True,
        })])
        self.assertEqual(result["rpcs"][0]["body_types"], {"object": 1})
        safe = summarizer.summarize([record(contentType="application/json; password=SECRET")])
        self.assertEqual(safe["rpcs"][0]["content_types"], {"application/json": 1})
        self.assertNotIn("SECRET", json.dumps(safe))


if __name__ == "__main__":
    unittest.main()
