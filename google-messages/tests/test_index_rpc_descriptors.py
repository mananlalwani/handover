import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).parents[1] / "tools" / "index_rpc_descriptors.py"
SPEC = importlib.util.spec_from_file_location("index_rpc_descriptors", MODULE_PATH)
indexer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(indexer)


def write_capture(directory: Path, script: bytes, filename: str = "script-01.js") -> None:
    directory.mkdir()
    (directory / filename).write_bytes(script)
    (directory / "manifest.json").write_text(json.dumps({
        "scripts": [{
            "file": filename,
            "size": len(script),
            "sha256": hashlib.sha256(script).hexdigest(),
        }]
    }))


class IndexRpcDescriptorsTests(unittest.TestCase):
    def test_indexes_strict_literals_and_reports_unindexed_candidates(self):
        source = (
            'x(); new _.jH("/google.internal.communications.instantmessaging.v1.Service/Get",Req,_.Reply,{});\n'
            'new _.jH("/google.internal.communications.instantmessaging.v1.ServiceB/Put",_.Input,Output,{});\n'
            '"google.internal.communications.instantmessaging.v1.ServiceC/Other";'
        )
        with tempfile.TemporaryDirectory() as temp:
            capture_dir = Path(temp) / "capture"
            write_capture(capture_dir, source.encode())
            result = indexer.index_capture(capture_dir)
        self.assertEqual(result["candidate_count"], 3)
        self.assertEqual(result["indexed_count"], 2)
        self.assertFalse(result["complete"])
        self.assertEqual(result["records"][0], {
            "rpc_path": "/google.internal.communications.instantmessaging.v1.Service/Get",
            "request_symbol": "Req",
            "response_symbol": "_.Reply",
            "script_file": "script-01.js",
            "script_sha256": hashlib.sha256(source.encode()).hexdigest(),
            "source_character_offset": source.index("/google.internal.communications.instantmessaging.v1.Service/Get"),
        })

    def test_rejects_bad_json_hash_and_size(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            bad_json = root / "bad-json"
            bad_json.mkdir()
            (bad_json / "manifest.json").write_text("{")
            with self.assertRaises(indexer.IndexError):
                indexer.index_capture(bad_json)

            mismatch = root / "mismatch"
            write_capture(mismatch, b"new _.jH(\"/google.internal.communications.instantmessaging.v1.B/C\",A,B,{});")
            manifest_path = mismatch / "manifest.json"
            manifest = json.loads(manifest_path.read_text())
            manifest["scripts"][0]["sha256"] = "0" * 64
            manifest_path.write_text(json.dumps(manifest))
            with self.assertRaisesRegex(indexer.IndexError, "SHA-256"):
                indexer.index_capture(mismatch)

            manifest["scripts"][0]["sha256"] = hashlib.sha256(
                (mismatch / "script-01.js").read_bytes()
            ).hexdigest()
            manifest["scripts"][0]["size"] += 1
            manifest_path.write_text(json.dumps(manifest))
            with self.assertRaisesRegex(indexer.IndexError, "size mismatch"):
                indexer.index_capture(mismatch)

    def test_rejects_absolute_traversal_and_symlink_script_paths(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for name, filename in (("absolute", str(root / "external.js")),
                                   ("traversal", "../external.js")):
                capture_dir = root / name
                write_capture(capture_dir, b"", filename)
                with self.assertRaises(indexer.IndexError):
                    indexer.index_capture(capture_dir)

            capture_dir = root / "symlink"
            capture_dir.mkdir()
            outside = root / "outside.js"
            outside.write_bytes(b"public")
            (capture_dir / "linked.js").symlink_to(outside)
            (capture_dir / "manifest.json").write_text(json.dumps({"scripts": [{
                "file": "linked.js", "size": 6,
                "sha256": hashlib.sha256(b"public").hexdigest(),
            }]}))
            with self.assertRaisesRegex(indexer.IndexError, "symlink"):
                indexer.index_capture(capture_dir)

    def test_enforces_manifest_script_count_and_byte_bounds(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            oversized_manifest = root / "manifest-large"
            oversized_manifest.mkdir()
            (oversized_manifest / "manifest.json").write_bytes(
                b" " * (indexer.MANIFEST_LIMIT + 1)
            )
            with self.assertRaisesRegex(indexer.IndexError, "manifest.json exceeds"):
                indexer.index_capture(oversized_manifest)

            too_many = root / "too-many"
            too_many.mkdir()
            entries = [{"file": f"{i}.js", "size": 0, "sha256": hashlib.sha256(b"").hexdigest()}
                       for i in range(indexer.SCRIPT_COUNT_LIMIT + 1)]
            (too_many / "manifest.json").write_text(json.dumps({"scripts": entries}))
            with self.assertRaisesRegex(indexer.IndexError, "more than"):
                indexer.index_capture(too_many)

            too_large = root / "script-large"
            too_large.mkdir()
            data = b"x" * (indexer.SCRIPT_LIMIT + 1)
            (too_large / "script.js").write_bytes(data)
            (too_large / "manifest.json").write_text(json.dumps({"scripts": [{
                "file": "script.js", "size": len(data), "sha256": hashlib.sha256(data).hexdigest(),
            }]}))
            with self.assertRaisesRegex(indexer.IndexError, "exceeds 8388608"):
                indexer.index_capture(too_large)

            if hasattr(Path, "is_fifo"):
                fifo_dir = root / "fifo"
                fifo_dir.mkdir()
                (fifo_dir / "manifest.json").write_text(json.dumps({"scripts": [{
                    "file": "pipe", "size": 0,
                    "sha256": hashlib.sha256(b"").hexdigest(),
                }]}))
                (fifo_dir / "pipe").parent.mkdir(exist_ok=True)
                import os
                os.mkfifo(fifo_dir / "pipe")
                with self.assertRaisesRegex(indexer.IndexError, "regular file"):
                    indexer.index_capture(fifo_dir)

    def test_enforces_total_script_bound_and_ignores_non_descriptor_rpc(self):
        with tempfile.TemporaryDirectory() as temp:
            capture_dir = Path(temp) / "total"
            capture_dir.mkdir()
            per_script = indexer.SCRIPT_LIMIT
            entries = []
            for i in range(indexer.SCRIPT_COUNT_LIMIT):
                filename = f"script-{i}.js"
                data = b"x" * per_script
                (capture_dir / filename).write_bytes(data)
                entries.append({"file": filename, "size": len(data),
                                "sha256": hashlib.sha256(data).hexdigest()})
            (capture_dir / "manifest.json").write_text(json.dumps({"scripts": entries}))
            with self.assertRaisesRegex(indexer.IndexError, "total limit"):
                indexer.index_capture(capture_dir)

        with tempfile.TemporaryDirectory() as temp:
            capture_dir = Path(temp) / "candidate"
            write_capture(capture_dir, b'"/google.internal.communications.instantmessaging.v1.Service/Call"')
            result = indexer.index_capture(capture_dir)
        self.assertEqual(result["candidate_count"], 1)
        self.assertEqual(result["indexed_count"], 0)


if __name__ == "__main__":
    unittest.main()
