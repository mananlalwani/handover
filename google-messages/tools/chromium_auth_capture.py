"""Bounded, single-request read-only authentication capture for the setup probe."""

import json
import os
import select
import struct
import subprocess
import time
from urllib.parse import urlsplit

ENDPOINTS = {
    "instantmessaging-pa.googleapis.com",
    "instantmessaging-pa.clients6.google.com",
    "instantmessaging-pa-jms-us.clients6.google.com",
}
SIGN_IN_PATH = "/$rpc/google.internal.communications.instantmessaging.v1.Registration/SignInGaia"
LIMITS = {"authorization": 8192, "x-goog-api-key": 4096, "cookie": 16384,
          "origin": 128, "x-goog-authuser": 32}


def endpoint_for(request):
    url = urlsplit(request.get("url", ""))
    if (request.get("method") != "POST" or url.scheme != "https"
            or url.hostname not in ENDPOINTS or url.path != SIGN_IN_PATH
            or url.username is not None or url.password is not None
            or url.port not in (None, 443)):
        return None
    return "https://" + url.hostname


def selected_headers(headers, include_cookie):
    selected = {}
    for key, value in headers.items():
        name = key.lower()
        if name not in LIMITS or (name == "cookie" and not include_cookie):
            continue
        if not isinstance(value, str) or not value or len(value) > LIMITS[name]:
            raise RuntimeError("Invalid authentication header")
        if "\r" in value or "\n" in value:
            raise RuntimeError("Invalid authentication header")
        selected[name] = value
    return selected


class Pipe:
    def __init__(self, reader, writer):
        self.reader, self.writer = reader, writer
        self.pending = b""
        self.next_id = 10
        self.deadline = time.monotonic() + 120

    def send(self, method, params=None, session=None):
        ident = self.next_id
        self.next_id += 1
        message = {"id": ident, "method": method, "params": params or {}}
        if session:
            message["sessionId"] = session
        payload = json.dumps(message).encode() + b"\0"
        while payload:
            payload = payload[os.write(self.writer, payload):]
        return ident

    def read(self):
        while b"\0" not in self.pending:
            if time.monotonic() >= self.deadline:
                raise RuntimeError("Capture timeout")
            if not select.select([self.reader], [], [], 0.5)[0]:
                continue
            chunk = os.read(self.reader, 65536)
            if not chunk:
                raise RuntimeError("Browser closed")
            self.pending += chunk
            if len(self.pending) > 256 * 1024:
                raise RuntimeError("Browser event exceeds bound")
        payload, self.pending = self.pending.split(b"\0", 1)
        return json.loads(payload)

    def call(self, method, params=None, session=None):
        ident = self.send(method, params, session)
        while True:
            message = self.read()
            if message.get("id") == ident:
                if "error" in message:
                    raise RuntimeError("Browser command failed")
                return message["result"]


def capture_proof(reader, writer, include_account=False):
    pipe = Pipe(reader, writer)
    targets = pipe.call("Target.getTargets")["targetInfos"]
    pages = [target for target in targets if target.get("type") == "page"
             and target.get("url") == "about:blank"]
    if len(pages) != 1:
        raise RuntimeError("Unexpected setup browser targets")
    session = pipe.call("Target.attachToTarget", {
        "targetId": pages[0]["targetId"], "flatten": True})["sessionId"]
    pipe.call("Network.enable", {"maxTotalBufferSize": 0, "maxResourceBufferSize": 0}, session)
    # No response bodies, cookie jar, or request postData are requested.
    pipe.send("Page.navigate", {"url": "https://messages.google.com/web/"}, session)
    candidates = {}
    proof = None
    try:
        while proof is None:
            message = pipe.read()
            if message.get("sessionId") != session:
                continue
            params = message.get("params", {})
            request_id = params.get("requestId")
            if message.get("method") == "Network.requestWillBeSent":
                endpoint = endpoint_for(params.get("request", {}))
                if endpoint is None:
                    candidates.pop(request_id, None)
                    continue
                if len(candidates) >= 4:
                    raise RuntimeError("Too many sign-in requests")
                candidates[request_id] = (endpoint, selected_headers(
                    params["request"].get("headers", {}), False))
            elif (message.get("method") == "Network.requestWillBeSentExtraInfo"
                  and request_id in candidates):
                endpoint, headers = candidates.pop(request_id)
                headers.update(selected_headers(params.get("headers", {}), True))
                if headers.get("origin", "https://messages.google.com") != "https://messages.google.com":
                    raise RuntimeError("Unexpected sign-in origin")
                if not all(name in headers for name in ("authorization", "x-goog-api-key", "cookie")):
                    raise RuntimeError("Required authentication header unavailable")
                proof = {"type": "gaia_lookup_with_cookies", "endpoint": endpoint,
                         "origin": "https://messages.google.com",
                         "authorization": headers["authorization"],
                         "api_key": headers["x-goog-api-key"],
                         "service_cookie": headers["cookie"]}
                if "x-goog-authuser" in headers:
                    proof["auth_user"] = headers["x-goog-authuser"]
                headers.clear()
        if include_account:
            email = read_account(pipe, session)
            if read_account(pipe, session) != email:
                raise RuntimeError("Signed-in account changed")
            proof["account_email"] = email
            proof["type"] = "gaia_pairing_start"
        result = proof
        proof = None
        return result
    finally:
        candidates.clear()
        if proof is not None:
            proof.clear()
        pipe.pending = b""


def read_account(pipe, session):
    # Read only the provider's account routing field, not the rest of the page.
    expression = r'''(()=>{try{if(location.origin!=="https://messages.google.com")return null;
const c=globalThis.MW_CONFIG;if(typeof c!=="string"||c.length>1048576)return null;
const d=JSON.parse(c);const f=(a,n)=>{if(!Array.isArray(a))return null;
const i=n-1,l=a.length-1,t=a[l];if(i<l)return a[i];
if(t&&typeof t==="object"&&!Array.isArray(t))return Object.hasOwn(t,n)?t[n]:null;
return i===l?t:null};return f(f(d,5),2)}catch{return null}})()'''
    response = pipe.call("Runtime.evaluate", {"expression": expression,
        "returnByValue": True, "awaitPromise": False, "silent": True}, session)
    email = response.get("result", {}).get("value")
    if not isinstance(email, str) or len(email) > 254 or "@" not in email:
        raise RuntimeError("Signed-in account unavailable")
    return email


def capture(reader, writer, native_probe):
    proof = capture_proof(reader, writer)
    try:
        payload = json.dumps(proof).encode()
        if len(payload) > 32 * 1024:
            raise RuntimeError("Authentication proof exceeds bound")
        result = subprocess.run([native_probe, "--local-read-only"],
                                input=struct.pack("=I", len(payload)) + payload,
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=30,
                                check=False)
        if result.returncode or len(result.stdout) < 4 or len(result.stdout) > 1028:
            raise RuntimeError("Native probe failed")
        length = struct.unpack("=I", result.stdout[:4])[0]
        if length != len(result.stdout) - 4:
            raise RuntimeError("Invalid native response")
        response = json.loads(result.stdout[4:])
        if response.get("ok") is True and isinstance(response.get("sources"), int):
            print("Native read-only authentication succeeded. No credentials saved.", flush=True)
        else:
            print("Native read-only authentication failed. No credentials saved.", flush=True)
    finally:
        proof.clear()
