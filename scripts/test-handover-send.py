#!/usr/bin/env python3
"""Exercise the send window through a local daemon fixture without a phone."""

import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import threading

ROOT = Path(__file__).resolve().parent.parent


def check_window(disconnect=False, accept_before_disconnect=False):
    with tempfile.TemporaryDirectory(prefix="handover-send-") as directory:
        root = Path(directory)
        runtime = root / "runtime"
        (runtime / "handover").mkdir(parents=True)
        runtime.chmod(0o700)
        for name in ("send.qml", "HandoverService.qml", "Share.js"):
            shutil.copyfile(ROOT / "quickshell" / name, root / name)
        expected = "outcome is unknown" if disconnect else "Transfer completed."
        qml = (root / "send.qml").read_text()
        harness = '''
    Timer {
        interval: 20
        running: true
        repeat: true
        property bool submitted: false
        onTriggered: {
            if (!submitted && HandoverService.connected && window.choices.length > 0) {
                submitted = true;
                window.submit();
            }
            if (window.transfer && window.transfer.phase === "done") {
                console.log("SEND_FIXTURE_RESULT " + window.transfer.text);
                Qt.quit();
            }
        }
    }
'''
        (root / "send.qml").write_text(qml.replace("ShellRoot {", "ShellRoot {" + harness, 1))
        requests = []
        failures = []
        with socket.socket(socket.AF_UNIX) as listener:
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

                        with conn.makefile("rb") as stream:
                            for line in stream:
                                request = json.loads(line)
                                if request["method"] == "devices.list":
                                    send(type="devices", devices=[dict(id="fixture", name="Fixture phone",
                                         paired=True, connected=True, capabilities=["file_transfer"])])
                                if request["method"] == "share.file":
                                    requests.append(request)
                                    if disconnect:
                                        if accept_before_disconnect:
                                            send(type="share_accepted", device_id="fixture", transfer_id="transfer")
                                        return
                                    # Transient completion can race ahead of acceptance.
                                    send(type="share_result", result=dict(device_id="fixture",
                                         transfer_id="transfer", status="completed"))
                                    send(type="share_accepted", device_id="fixture", transfer_id="transfer")
                except Exception as error:
                    failures.append(error)

            worker = threading.Thread(target=daemon, daemon=True)
            worker.start()
            environment = dict(os.environ, QT_QPA_PLATFORM="offscreen", XDG_RUNTIME_DIR=str(runtime),
                               XDG_CACHE_HOME=str(root), HANDOVER_SHARE_RESOURCE="/tmp/a #✓.txt")
            environment.pop("WAYLAND_DISPLAY", None)
            result = subprocess.run(["quickshell", "--path", str(root / "send.qml")],
                                    env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                    text=True, timeout=12)
            worker.join(timeout=1)
            assert not failures, failures
            assert result.returncode == 0 and "SEND_FIXTURE_RESULT " in result.stdout, result.stdout
            assert expected in result.stdout, result.stdout
            assert len(requests) == 1, requests
            assert requests[0]["device_id"] == "fixture", requests
            assert requests[0]["file_url"] == "file:///tmp/a%20%23%E2%9C%93.txt", requests
            assert not any(word in result.stdout for word in ("ReferenceError", "TypeError", "Binding loop")), result.stdout


if __name__ == "__main__":
    check_window(disconnect=False)
    check_window(disconnect=True)
    check_window(disconnect=True, accept_before_disconnect=True)
    print("Send window fixture tests passed")
