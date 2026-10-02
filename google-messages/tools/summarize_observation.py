#!/usr/bin/env python3
"""Validate and summarize a sanitized Chrome RPC observation array."""

from __future__ import annotations

import argparse
import json
import os
import re
import stat
import sys
from collections import Counter
from pathlib import Path


FILE_LIMIT = 16 * 1024 * 1024
RECORD_LIMIT = 512
NODE_LIMIT = 300_000
DEPTH_LIMIT = 12
OBJECT_FIELD_LIMIT = 512
ARRAY_ITEM_LIMIT = 128
NUMBER_LIMIT = FILE_LIMIT
RECORD_KEYS = {"type", "service", "method", "httpMethod", "status", "contentType", "body"}
HTTP_METHODS = {"GET", "POST", "OPTIONS"}
CONTENT_TYPE_RE = re.compile(r"^[A-Za-z0-9!#$&^_.+-]+/[A-Za-z0-9!#$&^_.+-]+$")
WIRE_TYPES = {0, 1, 2, 5}
CONTENT_TYPES = {
    "application/json", "application/json+protobuf",
    "application/protobuf", "application/x-protobuf",
}
PROTOBUF_FIELD_LIMIT = 536_870_911

# Literal public bootstrap candidates. This list does not imply that any method
# is safe, authenticated, supported, or callable.
KNOWN_METHODS = {
    "Messaging/AckMessages", "Messaging/Echo", "Messaging/PrewarmReceiver",
    "Messaging/PullMessages", "Messaging/ReceiveMessages", "Messaging/SendMessage",
    "Pairing/GetWebEncryptionKey", "Pairing/RefreshPhoneRelay",
    "Pairing/RegisterPhoneRelay", "Pairing/RevokeRelayPairing",
    "Registration/DeleteAccount", "Registration/GetAccountInfo",
    "Registration/LinkIdentity", "Registration/ListIdentities",
    "Registration/LookupRegistered", "Registration/Register",
    "Registration/RegisterRefresh", "Registration/SignInGaia",
    "Registration/SignInSecondary", "Registration/Unregister",
    "SmartMessaging/CreateConversation", "SmartMessaging/GetContentDecoration",
}


class ObservationError(Exception):
    """An observation did not satisfy the bounded schema."""


def _read_input(path: Path) -> bytes:
    flags = os.O_RDONLY | getattr(os, "O_NONBLOCK", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(path, flags)
    except OSError as exc:
        raise ObservationError("input_file_unavailable") from exc
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise ObservationError("input_not_regular_file")
        with os.fdopen(fd, "rb") as stream:
            fd = -1
            data = stream.read(FILE_LIMIT + 1)
    finally:
        if fd >= 0:
            os.close(fd)
    if len(data) > FILE_LIMIT:
        raise ObservationError("input_too_large")
    return data


def _bounded_int(value: object, *, minimum: int = 0, maximum: int = NUMBER_LIMIT) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and minimum <= value <= maximum


class _BodyValidator:
    def __init__(self) -> None:
        self.nodes = 0

    def _node(self, node: object, depth: int) -> tuple[str, int | None]:
        self.nodes += 1
        if self.nodes > NODE_LIMIT:
            raise ObservationError("body_node_limit")
        if depth > DEPTH_LIMIT:
            raise ObservationError("body_depth_limit")
        if not isinstance(node, dict) or not isinstance(node.get("type"), str):
            raise ObservationError("invalid_body_node")
        kind = node["type"]
        allowed: dict[str, set[str]] = {
            "array": {"type", "length", "items", "truncated"},
            "object": {"type", "propertyCount", "values", "truncated"},
            "string": {"type", "bytes"},
            "number": {"type"},
            "boolean": {"type"},
            "null": {"type"},
            "opaque": {"type", "bytes", "representationBytes", "truncated"},
            "unavailable": {"type"},
            "truncated": {"type"},
            "protobuf_wire": {"type", "bytes", "fields", "truncated"},
        }
        if kind not in allowed or set(node) - allowed[kind]:
            raise ObservationError("invalid_body_node")
        if "truncated" in node and not isinstance(node["truncated"], bool):
            raise ObservationError("invalid_body_node")

        if kind == "array":
            length, items = node.get("length"), node.get("items")
            if node.get("truncated") is True and "length" not in node and "items" not in node:
                return kind, None
            if not _bounded_int(length) or not isinstance(items, list) or len(items) > ARRAY_ITEM_LIMIT or len(items) > length:
                raise ObservationError("invalid_body_node")
            for item in items:
                self._node(item, depth + 1)
            return kind, length
        if kind == "object":
            count, values = node.get("propertyCount"), node.get("values")
            if node.get("truncated") is True and "propertyCount" not in node and "values" not in node:
                return kind, None
            if not _bounded_int(count) or not isinstance(values, list) or len(values) > OBJECT_FIELD_LIMIT or len(values) > count:
                raise ObservationError("invalid_body_node")
            for value in values:
                self._node(value, depth + 1)
            return kind, None
        if kind == "string":
            if not _bounded_int(node.get("bytes")):
                raise ObservationError("invalid_body_node")
        elif kind == "opaque":
            for key in ("bytes", "representationBytes"):
                if key in node and not _bounded_int(node[key]):
                    raise ObservationError("invalid_body_node")
        elif kind == "protobuf_wire":
            if not _bounded_int(node.get("bytes"), maximum=1024 * 1024):
                raise ObservationError("invalid_body_node")
            fields = node.get("fields")
            if not isinstance(fields, list) or len(fields) > ARRAY_ITEM_LIMIT:
                raise ObservationError("invalid_body_node")
            for field in fields:
                self.nodes += 1
                if self.nodes > NODE_LIMIT:
                    raise ObservationError("body_node_limit")
                if not isinstance(field, dict) or not {"field", "wireType"} <= set(field):
                    raise ObservationError("invalid_body_node")
                wire_type = field["wireType"]
                expected_keys = {"field", "wireType", "bytes"} if wire_type == 2 else {"field", "wireType"}
                if (set(field) != expected_keys
                        or not _bounded_int(field["field"], minimum=1, maximum=PROTOBUF_FIELD_LIMIT)
                        or not _bounded_int(wire_type, maximum=5) or wire_type not in WIRE_TYPES
                        or (wire_type == 2 and not _bounded_int(field["bytes"], maximum=node["bytes"]))):
                    raise ObservationError("invalid_body_node")
        return kind, None

    def validate(self, node: object) -> tuple[str, int | None]:
        return self._node(node, 0)


def summarize(value: object) -> dict:
    if not isinstance(value, list):
        raise ObservationError("input_must_be_array")
    if len(value) > RECORD_LIMIT:
        raise ObservationError("record_limit")

    grouped: dict[str, dict[str, object]] = {}
    body_validator = _BodyValidator()
    for record in value:
        if not isinstance(record, dict) or set(record) - RECORD_KEYS:
            raise ObservationError("invalid_record")
        if not {"type", "service", "method"} <= set(record):
            raise ObservationError("invalid_record")
        phase, service, method = record["type"], record["service"], record["method"]
        if (not isinstance(phase, str) or phase not in {"request", "response"}
                or not isinstance(service, str) or not isinstance(method, str)):
            raise ObservationError("invalid_record")
        rpc = f"{service}/{method}"
        if rpc not in KNOWN_METHODS:
            raise ObservationError("unknown_rpc")
        entry = grouped.setdefault(rpc, {
            "rpc": rpc, "count": 0, "requests": 0,
            "http_methods": Counter(), "statuses": Counter(),
            "content_types": Counter(), "body_types": Counter(),
            "top_level_array_lengths": Counter(),
        })
        entry["count"] += 1
        if phase == "request":
            entry["requests"] += 1
        if "httpMethod" in record:
            http_method = record["httpMethod"]
            if not isinstance(http_method, str) or http_method not in HTTP_METHODS:
                raise ObservationError("invalid_http_method")
            entry["http_methods"][http_method] += 1
        if "status" in record:
            status = record["status"]
            if not _bounded_int(status, minimum=100, maximum=599):
                raise ObservationError("invalid_status")
            entry["statuses"][str(status)] += 1
        if "contentType" in record:
            content_type = record["contentType"]
            if not isinstance(content_type, str) or len(content_type) > 200:
                raise ObservationError("invalid_content_type")
            media_type = content_type.split(";", 1)[0].strip().lower()
            if not CONTENT_TYPE_RE.fullmatch(media_type) or media_type not in CONTENT_TYPES:
                raise ObservationError("invalid_content_type")
            entry["content_types"][media_type] += 1
        if "body" in record:
            body_type, array_length = body_validator.validate(record["body"])
            entry["body_types"][body_type] += 1
            if body_type == "array" and array_length is not None:
                entry["top_level_array_lengths"][str(array_length)] += 1

    result = []
    for rpc in sorted(grouped):
        entry = grouped[rpc]
        result.append({
            "rpc": rpc,
            "count": entry["count"],
            "requests": entry["requests"],
            "http_methods": dict(sorted(entry["http_methods"].items())),
            "statuses": dict(sorted(entry["statuses"].items(), key=lambda pair: int(pair[0]))),
            "content_types": dict(sorted(entry["content_types"].items())),
            "body_types": dict(sorted(entry["body_types"].items())),
            "top_level_array_lengths": dict(sorted(
                entry["top_level_array_lengths"].items(), key=lambda pair: int(pair[0])
            )),
        })
    return {"record_count": len(value), "rpcs": result}


def summarize_file(path: Path) -> dict:
    data = _read_input(Path(path))
    try:
        parsed = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError, ValueError) as exc:
        raise ObservationError("invalid_json") from exc
    return summarize(parsed)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("observation", type=Path)
    args = parser.parse_args(argv)
    try:
        result = summarize_file(args.observation)
    except ObservationError as exc:
        print(f"summarize_observation: {exc}", file=sys.stderr)
        return 1
    json.dump(result, sys.stdout, indent=2)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
