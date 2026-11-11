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
        for name in ("send.qml", "HandoverService.qml", "IpcRequest.qml", "Share.js"):
            shutil.copyfile(ROOT / "quickshell" / name, qml_root / name)
        expected = {
            "completed": "Transfer completed.",
            "unrelated-error": "Transfer completed.",
            "parallel-errors": "Transfer completed.",
            "wrong-reply": "outcome is unknown",
            "reconnect-no-replay": "outcome is unknown",
            "bounded-requests": "Request failed: too many requests",
            "disconnected": "outcome is unknown",
            "accepted-disconnected": "outcome is unknown",
            "rejected": "Request failed: device_disconnected",
            "cancelled": "Queued transfer cancelled.",
            "cancel-rejected": "Transfer completed.",
            "cancel-error": "Transfer completed.",
            "cancel-disconnected": "outcome is unknown",
            "cancel-reply-lost": "Transfer completed.",
            "close-accepted": "Closed with transfer pending.",
            "offline": "Sharing unavailable; no request sent.",
            "no-daemon": "Sharing unavailable; no request sent.",
        }[scenario]
        harness = '''
    Timer {
        id: sendFixture
        interval: 20
        running: true
        repeat: true
        property bool submitted: false
        property bool cancelRequested: false
        property bool uncertaintyObserved: false
        property bool sawDisconnect: false
        property bool filledRequests: false
        property bool messagingRejected: false
        property int ticks: 0
        readonly property string scenario: String(Quickshell.env("HANDOVER_SEND_SCENARIO"))
        onTriggered: {
            ticks++;
            if (submitted && !HandoverService.connected)
                sawDisconnect = true;
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
                if (scenario === "bounded-requests") {
                    if (!filledRequests && HandoverService.activeRequests.length === 0) {
                        filledRequests = true;
                        for (let index = 0; index < 16; index++) {
                            if (!HandoverService.loadHistory({account_id:"fixture",local_id:String(index)}, 20)) {
                                console.log("SEND_FIXTURE_ERROR admission failed before request limit");
                                Qt.exit(1);
                                return;
                            }
                        }
                    }
                    if (!HandoverService.devices.some(device => device.name === "Requests pending"))
                        return;
                }
                submitted = true;
                window.submit();
                // A second activation while pending must not send again.
                window.submit();
                if (scenario === "parallel-errors") {
                    HandoverService.sendText({account_id:"fixture",local_id:"thread"}, "fixture text", null);
                    HandoverService.nativeAction({id:"native:fixture"}, "ping");
                }
            }
            if (!uncertaintyObserved && (scenario === "unrelated-error" || scenario === "parallel-errors")
                    && HandoverService.lastError.includes("background history unavailable")) {
                if (!window.transfer || window.transfer.phase !== "pending") {
                    console.log("SEND_FIXTURE_ERROR background error resolved another request");
                    Qt.exit(1);
                    return;
                }
                if (scenario === "parallel-errors" && (!HandoverService.pendingMessaging || !HandoverService.nativePending)) {
                    console.log("SEND_FIXTURE_ERROR background error released another operation");
                    Qt.exit(1);
                    return;
                }
                uncertaintyObserved = true;
                HandoverService.refreshCallAudio();
            }
            if (!uncertaintyObserved && scenario === "cancel-reply-lost" && window.transfer
                    && window.transfer.text.includes("Cancellation outcome is unknown")) {
                uncertaintyObserved = true;
                HandoverService.refreshCallAudio();
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
                if (scenario === "parallel-errors" && (HandoverService.pendingMessaging || HandoverService.nativePending))
                    return;
                if (scenario === "parallel-errors" && !messagingRejected) {
                    console.log("SEND_FIXTURE_ERROR messaging rejection was not attributed");
                    Qt.exit(1);
                    return;
                }
                if (scenario === "reconnect-no-replay" && (!sawDisconnect
                        || !HandoverService.connected || window.choices.length === 0))
                    return;
                if ((scenario === "disconnected" || scenario === "rejected" || scenario === "wrong-reply")
                        && !HandoverService.connected) {
                    console.log("SEND_FIXTURE_ERROR command failure closed the subscription");
                    Qt.exit(1);
                    return;
                }
                console.log("SEND_FIXTURE_RESULT " + window.transfer.text);
                Qt.quit();
            }
        }
    }
    Connections {
        target: HandoverService
        function onMessagingAccepted(method, subject, success, error) {
            if (sendFixture.scenario === "parallel-errors") {
                if (method !== "messages.send" || success || !error.includes("fixture message rejected")) {
                    console.log("SEND_FIXTURE_ERROR incorrect messaging completion");
                    Qt.exit(1);
                    return;
                }
                sendFixture.messagingRejected = true;
            }
        }
    }
'''
        qml = (qml_root / "send.qml").read_text()
        (qml_root / "send.qml").write_text(qml.replace("ShellRoot {", "ShellRoot {" + harness, 1))
        requests = []
        failures = []
        with socket.socket(socket.AF_UNIX) as listener:
            stopped = threading.Event()
            stream_ready = threading.Event()
            share_seen = threading.Event()
            background_observed = threading.Event()
            stream_connection = [None]
            send_lock = threading.Lock()
            handlers = []
            connections = []
            histories = []
            history_lock = threading.Lock()

            def send_to(conn, **fields):
                with send_lock:
                    conn.sendall((json.dumps(dict(protocol=1, **fields)) + "\n").encode())

            def event(**fields):
                assert stream_ready.wait(3), "subscription was not established"
                send_to(stream_connection[0], **fields)

            def result(status="completed", **fields):
                event(type="share_result", result=dict(device_id="fixture",
                      transfer_id=TRANSFER, status=status, **fields))

            device = dict(id="fixture", name="Fixture phone", paired=True,
                          connected=scenario != "offline", capabilities=["file_transfer"])

            def handle(conn):
                methods = []
                connections.append(methods)
                try:
                    with conn:
                        conn.settimeout(10)
                        with conn.makefile("rb") as incoming:
                            for line in incoming:
                                request = json.loads(line)
                                method = request["method"]
                                methods.append(method)
                                if method == "hello":
                                    send_to(conn, type="hello", supported_protocols=[1])
                                elif method == "subscribe":
                                    stream_connection[0] = conn
                                    send_to(conn, type="subscribed", devices=[device], notifications=[],
                                            media_sessions=[], calls=[], messaging_accounts=[],
                                            conversations=[], typing_states=[], read_states=[])
                                    stream_ready.set()
                                elif method == "audio.subscribe":
                                    send_to(conn, type="audio_streams", streams=[])
                                elif method == "contacts.list":
                                    send_to(conn, type="contacts", contacts=[])
                                elif method == "messages.archive_settings":
                                    if scenario in ("unrelated-error", "parallel-errors"):
                                        assert share_seen.wait(3), "share was not submitted"
                                        send_to(conn, type="error", code="history_unavailable",
                                                message="background history unavailable")
                                    else:
                                        send_to(conn, type="archive_settings", limit_mib=1024, used_bytes=0)
                                elif method == "native.pending":
                                    send_to(conn, type="native_pending", pending=[])
                                elif method == "custom.commands":
                                    send_to(conn, type="custom_commands", commands=[])
                                elif method == "clipboard.mirror_status":
                                    send_to(conn, type="clipboard_mirror", enabled=False)
                                elif method == "clipboard.history_list":
                                    send_to(conn, type="clipboard_history", entries=[])
                                elif method == "messages.outgoing":
                                    send_to(conn, type="outgoing_operations", operations=[], done=True)
                                elif method == "calls.audio":
                                    background_observed.set()
                                    send_to(conn, type="call_audio", status=dict(observed=False,
                                            gateway_ready=False, duplex_running=False))
                                elif method in ("messages.send", "native.ping") and scenario == "parallel-errors":
                                    assert background_observed.wait(3), "UI did not observe pending commands"
                                    if method == "messages.send":
                                        send_to(conn, type="error", code="backend_rejected", message="fixture message rejected")
                                    else:
                                        send_to(conn, type="native_accepted")
                                elif method == "messages.history" and scenario == "bounded-requests":
                                    with history_lock:
                                        histories.append(request)
                                        full = len(histories) == 16
                                    if full:
                                        event(type="device_updated", device=dict(device, name="Requests pending"))
                                    stopped.wait(5)
                                    return
                                elif method in ("share.file", "share.url"):
                                    requests.append(request)
                                    share_seen.set()
                                    if scenario == "disconnected":
                                        return
                                    if scenario == "rejected":
                                        send_to(conn, type="error", code="device_disconnected",
                                                message="device is disconnected")
                                        continue
                                    if scenario == "wrong-reply":
                                        send_to(conn, type="share_accepted", device_id="another-device", transfer_id=TRANSFER)
                                        continue
                                    if scenario in ("unrelated-error", "parallel-errors"):
                                        assert background_observed.wait(3), "UI did not observe the unrelated error"
                                    if scenario in ("completed", "unrelated-error", "parallel-errors"):
                                        # Events belong to the subscription; replies belong to this socket.
                                        result()
                                    send_to(conn, type="share_accepted", device_id="fixture", transfer_id=TRANSFER)
                                    if scenario in ("accepted-disconnected", "reconnect-no-replay"):
                                        stream_connection[0].shutdown(socket.SHUT_RDWR)
                                elif method == "share.cancel":
                                    requests.append(request)
                                    if scenario == "cancel-disconnected":
                                        stream_connection[0].shutdown(socket.SHUT_RDWR)
                                        return
                                    if scenario == "cancel-reply-lost":
                                        conn.shutdown(socket.SHUT_RDWR)
                                        assert background_observed.wait(3), "UI did not preserve cancellation uncertainty"
                                        result()
                                        return
                                    if scenario == "cancelled":
                                        result("failed", reason="rejected")
                                        send_to(conn, type="share_cancelled", cancelled=True)
                                    elif scenario == "cancel-rejected":
                                        send_to(conn, type="share_cancelled", cancelled=False)
                                        result()
                                    elif scenario == "cancel-error":
                                        send_to(conn, type="error", code="backend_unavailable", message="backend unavailable")
                                        result()
                                else:
                                    raise AssertionError(f"unexpected fixture request: {method}")
                except (BrokenPipeError, ConnectionResetError):
                    pass  # The UI closes unused background queries when the window exits.
                except Exception as error:
                    failures.append(error)

            def daemon():
                while not stopped.is_set():
                    try:
                        conn, _ = listener.accept()
                    except socket.timeout:
                        continue
                    handler = threading.Thread(target=handle, args=(conn,), daemon=True)
                    handlers.append(handler)
                    handler.start()

            worker = None
            if scenario != "no-daemon":
                listener.bind(str(runtime / "handover" / "handoverd.sock"))
                listener.listen(32)
                listener.settimeout(0.1)
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
            try:
                process = subprocess.run(command, env=environment, stdout=subprocess.PIPE,
                                         stderr=subprocess.STDOUT, text=True, timeout=12)
            except subprocess.TimeoutExpired as error:
                stopped.set()
                output = error.output.decode() if isinstance(error.output, bytes) else error.output
                raise AssertionError(f"{scenario}: {failures}\n{connections}\n{output}") from error
            stopped.set()
            if worker:
                worker.join(timeout=1)
                for handler in handlers:
                    handler.join(timeout=2)
                    assert not handler.is_alive(), "request fixture did not terminate"
                assert not worker.is_alive(), "daemon fixture did not terminate"
            assert not failures, (failures, connections, process.stdout)
            for methods in connections:
                if "subscribe" in methods:
                    assert set(methods) <= {"hello", "subscribe", "audio.subscribe"}, methods
                else:
                    assert len(methods) == 1, methods
            assert process.returncode == 0 and "SEND_FIXTURE_RESULT " in process.stdout, process.stdout
            assert expected in process.stdout, process.stdout
            shares = [request for request in requests if request["method"] != "share.cancel"]
            cancels = [request for request in requests if request["method"] == "share.cancel"]
            unavailable = scenario in ("offline", "no-daemon", "bounded-requests")
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
            if scenario == "bounded-requests":
                assert len(histories) == 16, histories
            if scenario == "reconnect-no-replay":
                assert sum("subscribe" in methods for methods in connections) == 2, connections
            assert not any(word in process.stdout for word in (
                "ReferenceError", "TypeError", "Binding loop", "Quickshell has crashed", "SEND_FIXTURE_ERROR")), process.stdout


if __name__ == "__main__":
    for scenario in ("completed", "unrelated-error", "parallel-errors", "wrong-reply", "reconnect-no-replay", "bounded-requests",
                     "disconnected", "accepted-disconnected", "rejected", "cancelled",
                     "cancel-rejected", "cancel-error", "cancel-disconnected", "cancel-reply-lost", "close-accepted", "offline", "no-daemon"):
        check_window(scenario)
    for resource in ("https://example.test/a%20b?q=1#part", "http://example.test/✓?q=%25",
                     "file:///tmp/a%20%23%E2%9C%93.txt"):
        check_window("completed", resource)
    print("Send window fixture tests passed: 20 desktop entry and failure scenarios")
