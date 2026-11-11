#!/usr/bin/env python3
"""Exercise the desktop entry, launcher, and send window without a phone."""

import configparser
import json
import os
from pathlib import Path
import shlex
import shutil
import socket
import subprocess
import tempfile
import threading

ROOT = Path(__file__).resolve().parent.parent
TRANSFER = "00112233445566778899aabbccddeeff"


def check_window(scenario, resource="/tmp/a #?%\n✓.txt"):
    with tempfile.TemporaryDirectory(prefix="handover-send-") as directory:
        root = Path(directory)
        runtime = root / "runtime"
        (runtime / "handover").mkdir(parents=True)
        runtime.chmod(0o700)
        qml_root = root / "share/handover/quickshell"
        qml_root.mkdir(parents=True)
        for name in ("send.qml", "HandoverService.qml", "Share.js"):
            shutil.copyfile(ROOT / "quickshell" / name, qml_root / name)
        expected = {
            "completed": "Transfer completed.",
            "disconnected": "outcome is unknown",
            "accepted-disconnected": "outcome is unknown",
            "rejected": "Request failed: device_disconnected",
            "cancelled": "Queued transfer cancelled.",
            "cancel-rejected": "Transfer completed.",
            "cancel-error": "Transfer completed.",
            "cancel-disconnected": "outcome is unknown",
            "close-accepted": "Closed with transfer pending.",
            "offline": "Sharing unavailable; no request sent.",
            "no-daemon": "Sharing unavailable; no request sent.",
        }[scenario]
        harness = '''
    Timer {
        interval: 20
        running: true
        repeat: true
        property bool submitted: false
        property bool cancelRequested: false
        property int ticks: 0
        readonly property string scenario: String(Quickshell.env("HANDOVER_SEND_SCENARIO"))
        onTriggered: {
            ticks++;
            if ((scenario === "no-daemon" && ticks > 10)
                    || (scenario === "offline" && HandoverService.devices.length > 0)) {
                window.submit();
                if (window.transfer || window.choices.length > 0) {
                    console.log("SEND_FIXTURE_ERROR unavailable device allowed submission");
                    Qt.exit(1);
                    return;
                }
                console.log("SEND_FIXTURE_RESULT Sharing unavailable; no request sent.");
                Qt.quit();
                return;
            }
            if (!submitted && HandoverService.connected && window.choices.length > 0) {
                submitted = true;
                window.submit();
                // A second activation while pending must not send again.
                window.submit();
            }
            if (window.transfer && window.transfer.phase === "accepted") {
                if (scenario === "close-accepted") {
                    console.log("SEND_FIXTURE_RESULT Closed with transfer pending.");
                    Qt.quit();
                    return;
                }
                if ((scenario === "cancelled" || scenario.startsWith("cancel-")) && !cancelRequested) {
                    cancelRequested = true;
                    window.cancelTransfer();
                }
            }
            if (window.transfer && window.transfer.phase === "done" && !window.transfer.cancelPending) {
                console.log("SEND_FIXTURE_RESULT " + window.transfer.text);
                Qt.quit();
            }
        }
    }
'''
        qml = (qml_root / "send.qml").read_text()
        (qml_root / "send.qml").write_text(qml.replace("ShellRoot {", "ShellRoot {" + harness, 1))
        requests = []
        failures = []
        with socket.socket(socket.AF_UNIX) as listener:
            if scenario != "no-daemon":
                listener.bind(str(runtime / "handover" / "handoverd.sock"))
                listener.listen(1)
                listener.settimeout(10)

            def daemon():
                try:
                    conn, _ = listener.accept()
                    with conn:
                        conn.settimeout(10)

                        def send(**fields):
                            conn.sendall((json.dumps(dict(protocol=1, **fields)) + "\n").encode())

                        def result(status="completed", **fields):
                            send(type="share_result", result=dict(device_id="fixture",
                                 transfer_id=TRANSFER, status=status, **fields))

                        with conn.makefile("rb") as stream:
                            for line in stream:
                                request = json.loads(line)
                                method = request["method"]
                                if method == "devices.list":
                                    send(type="devices", devices=[dict(id="fixture", name="Fixture phone",
                                         paired=True, connected=scenario != "offline", capabilities=["file_transfer"])])
                                if method in ("share.file", "share.url"):
                                    requests.append(request)
                                    if scenario == "disconnected":
                                        return
                                    if scenario == "rejected":
                                        send(type="error", code="device_disconnected", message="device is disconnected")
                                        continue
                                    if scenario == "completed":
                                        # Transient completion can race ahead of acceptance.
                                        result()
                                    send(type="share_accepted", device_id="fixture", transfer_id=TRANSFER)
                                    if scenario == "accepted-disconnected":
                                        return
                                if method == "share.cancel":
                                    requests.append(request)
                                    if scenario == "cancel-disconnected":
                                        return
                                    if scenario == "cancelled":
                                        # The daemon publishes rejection before its cancel reply.
                                        result("failed", reason="rejected")
                                        send(type="share_cancelled", cancelled=True)
                                    elif scenario == "cancel-rejected":
                                        send(type="share_cancelled", cancelled=False)
                                        result()
                                    elif scenario == "cancel-error":
                                        send(type="error", code="backend_unavailable", message="backend unavailable")
                                        result()
                except Exception as error:
                    failures.append(error)

            worker = None
            if scenario != "no-daemon":
                worker = threading.Thread(target=daemon, daemon=True)
                worker.start()
            environment = dict(os.environ, QT_QPA_PLATFORM="offscreen", XDG_RUNTIME_DIR=str(runtime),
                               XDG_DATA_HOME=str(root / "share"), XDG_CACHE_HOME=str(root),
                               HANDOVER_SEND_SCENARIO=scenario)
            environment.pop("WAYLAND_DISPLAY", None)
            # Use the real desktop entry's Exec and launcher, with an isolated data home.
            desktop = configparser.ConfigParser(interpolation=None)
            desktop.read(ROOT / "packaging/handover-send.desktop")
            entry = desktop["Desktop Entry"]
            if resource.startswith(("http://", "https://")):
                assert f"x-scheme-handler/{resource.split(':')[0]};" in entry["MimeType"]
            command = shlex.split(entry["Exec"])
            assert command == ["handover-gui", "--send", "%u"], command
            command[0] = str(ROOT / "scripts/handover-gui")
            command[-1] = resource
            process = subprocess.run(command, env=environment, stdout=subprocess.PIPE,
                                     stderr=subprocess.STDOUT, text=True, timeout=12)
            if worker:
                worker.join(timeout=1)
                assert not worker.is_alive(), "daemon fixture did not terminate"
            assert not failures, failures
            assert process.returncode == 0 and "SEND_FIXTURE_RESULT " in process.stdout, process.stdout
            assert expected in process.stdout, process.stdout
            shares = [request for request in requests if request["method"] != "share.cancel"]
            cancels = [request for request in requests if request["method"] == "share.cancel"]
            unavailable = scenario in ("offline", "no-daemon")
            assert len(shares) == (0 if unavailable else 1), requests
            if shares:
                assert shares[0]["device_id"] == "fixture", shares
                if resource.startswith(("http://", "https://")):
                    assert shares[0]["method"] == "share.url" and shares[0]["url"] == resource, shares
                else:
                    expected_url = resource if resource.startswith("file://") else Path(resource).as_uri()
                    assert shares[0]["file_url"] == expected_url, shares
            cancellation = scenario == "cancelled" or scenario.startswith("cancel-")
            assert len(cancels) == (1 if cancellation else 0), requests
            if cancels:
                assert cancels[0]["device_id"] == "fixture" and cancels[0]["transfer_id"] == TRANSFER, cancels
            assert not any(word in process.stdout for word in ("ReferenceError", "TypeError", "Binding loop")), process.stdout


if __name__ == "__main__":
    for scenario in ("completed", "disconnected", "accepted-disconnected", "rejected", "cancelled",
                     "cancel-rejected", "cancel-error", "cancel-disconnected", "close-accepted", "offline", "no-daemon"):
        check_window(scenario)
    for resource in ("https://example.test/a%20b?q=1#part", "http://example.test/✓?q=%25",
                     "file:///tmp/a%20%23%E2%9C%93.txt"):
        check_window("completed", resource)
    print("Send window fixture tests passed: 14 desktop entry and failure scenarios")
