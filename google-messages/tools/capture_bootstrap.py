#!/usr/bin/env python3
"""Capture anonymous Google Messages web bootstrap assets for offline inspection."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import sys
import urllib.error
import urllib.parse
import urllib.request
from html.parser import HTMLParser
from pathlib import Path


PAGE_URL = "https://messages.google.com/web/"
USER_AGENT = (
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36"
)
PAGE_LIMIT = 4 * 1024 * 1024
SCRIPT_LIMIT = 8 * 1024 * 1024
SCRIPT_COUNT_LIMIT = 8
TOTAL_LIMIT = 32 * 1024 * 1024
TIMEOUT_SECONDS = 30
RPC_PATTERN = re.compile(r"google\.internal\.[A-Za-z0-9_.]+/[A-Za-z0-9]+")


class CaptureError(Exception):
    """An input, URL, network, or size limit prevented a complete capture."""


def validate_url(url: str, kind: str) -> str:
    """Validate and return a canonical absolute URL for a page or script."""
    try:
        parsed = urllib.parse.urlsplit(url)
        port = parsed.port
    except ValueError as exc:
        raise CaptureError(f"invalid URL: {url!r}") from exc
    if parsed.scheme != "https" or parsed.username is not None or parsed.password is not None:
        raise CaptureError(f"URL must use HTTPS without credentials: {url!r}")
    if port not in (None, 443) or parsed.fragment:
        raise CaptureError(f"unsupported URL component: {url!r}")
    if kind == "page":
        allowed = parsed.hostname == "messages.google.com" and parsed.path == "/web/"
    elif kind == "script":
        allowed = (
            parsed.hostname == "www.gstatic.com"
            and parsed.path.startswith("/_/messagesweb/")
        )
    else:
        raise ValueError(f"unknown URL kind: {kind}")
    if not allowed:
        raise CaptureError(f"URL is outside the allowed {kind} origin/path: {url!r}")
    return urllib.parse.urlunsplit(("https", parsed.netloc, parsed.path, parsed.query, ""))


class ScriptSourceParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.sources: list[str] = []
        self.base_href: str | None = None

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        attributes = {name.lower(): value for name, value in attrs}
        if tag.lower() == "base" and self.base_href is None:
            self.base_href = attributes.get("href")
        elif tag.lower() == "script" and attributes.get("src") is not None:
            self.sources.append(attributes["src"])


class _SafeRedirectHandler(urllib.request.HTTPRedirectHandler):
    def __init__(self, kind: str) -> None:
        super().__init__()
        self.kind = kind

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        validate_url(newurl, self.kind)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def _fetch(url: str, kind: str, limit: int) -> bytes:
    safe_url = validate_url(url, kind)
    request = urllib.request.Request(
        safe_url,
        headers={"User-Agent": USER_AGENT, "Accept": "text/html,*/*"},
    )
    opener = urllib.request.build_opener(_SafeRedirectHandler(kind))
    try:
        with opener.open(request, timeout=TIMEOUT_SECONDS) as response:
            validate_url(response.geturl(), kind)
            data = response.read(limit + 1)
    except (urllib.error.URLError, TimeoutError, OSError) as exc:
        raise CaptureError(f"fetch failed for {safe_url}: {exc}") from exc
    if len(data) > limit:
        raise CaptureError(f"response exceeds {limit} byte limit: {safe_url}")
    return data


def capture(output: Path) -> tuple[int, int, int]:
    """Capture the page and permitted scripts into a new private directory."""
    output = Path(output)
    try:
        output.mkdir(mode=0o700, parents=False, exist_ok=False)
    except FileExistsError as exc:
        raise CaptureError(f"output directory already exists: {output}") from exc
    except OSError as exc:
        raise CaptureError(f"cannot create output directory {output}: {exc}") from exc

    try:
        page = _fetch(PAGE_URL, "page", PAGE_LIMIT)
        if len(page) > PAGE_LIMIT:
            raise CaptureError(f"page exceeds {PAGE_LIMIT} byte limit")
        parser = ScriptSourceParser()
        parser.feed(page.decode("utf-8", errors="replace"))
        base_url = urllib.parse.urljoin(PAGE_URL, parser.base_href) if parser.base_href else PAGE_URL
        urls: list[str] = []
        for source in parser.sources:
            absolute = urllib.parse.urljoin(base_url, source)
            safe = validate_url(absolute, "script")
            if safe not in urls:
                urls.append(safe)
        if len(urls) > SCRIPT_COUNT_LIMIT:
            raise CaptureError(f"page references more than {SCRIPT_COUNT_LIMIT} scripts")

        scripts: list[tuple[str, bytes]] = []
        total = len(page)
        for url in urls:
            body = _fetch(url, "script", SCRIPT_LIMIT)
            if len(body) > SCRIPT_LIMIT:
                raise CaptureError(f"script exceeds {SCRIPT_LIMIT} byte limit: {url}")
            total += len(body)
            if total > TOTAL_LIMIT:
                raise CaptureError(f"capture exceeds {TOTAL_LIMIT} byte total limit")
            scripts.append((url, body))

        _write_private(output / "page.html", page)
        manifest_scripts = []
        methods: set[str] = set()
        for index, (url, body) in enumerate(scripts, start=1):
            filename = f"script-{index:02d}.js"
            _write_private(output / filename, body)
            manifest_scripts.append({
                "file": filename,
                "source_url": url,
                "sha256": hashlib.sha256(body).hexdigest(),
                "size": len(body),
            })
            methods.update(RPC_PATTERN.findall(body.decode("utf-8", errors="replace")))
        manifest = {
            "page_source_url": PAGE_URL,
            "page": {
                "file": "page.html",
                "sha256": hashlib.sha256(page).hexdigest(),
                "size": len(page),
            },
            "scripts": manifest_scripts,
            "candidate_rpc_methods": sorted(methods),
        }
        _write_private(output / "manifest.json", json.dumps(manifest, indent=2).encode("utf-8") + b"\n")
        os.chmod(output, 0o700)
        return total, len(scripts), len(methods)
    except Exception:
        shutil.rmtree(output, ignore_errors=True)
        raise


def _write_private(path: Path, data: bytes) -> None:
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    fd = os.open(path, flags, 0o600)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
    except Exception:
        try:
            os.close(fd)
        except OSError:
            pass
        raise


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path, metavar="DIRECTORY")
    args = parser.parse_args(argv)
    try:
        byte_count, script_count, method_count = capture(args.output)
    except (CaptureError, OSError) as exc:
        print(f"capture_bootstrap: {exc}", file=sys.stderr)
        return 1
    print(f"{args.output} bytes={byte_count} scripts={script_count} methods={method_count}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
