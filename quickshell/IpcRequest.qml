import QtQml
import Quickshell
import Quickshell.Io

// One request on an unsubscribed socket. The owner assembles and validates replies.
Scope {
    id: request
    required property string socketPath
    required property string encodedRequest
    property bool submitted: false
    property bool finished: false
    signal received(string line)
    signal failed(string reason, bool submitted)

    function start() {
        if (finished)
            return;
        deadline.start();
        socket.connected = true;
    }

    function close() {
        if (finished)
            return;
        finished = true;
        deadline.stop();
        // Leave the socket's read/error callback before closing its connection.
        Qt.callLater(() => socket.connected = false);
    }

    function fail(reason) {
        if (finished)
            return;
        close();
        failed(reason, submitted);
    }

    Timer {
        id: deadline
        interval: 15000
        onTriggered: request.fail("request connection or response timed out")
    }

    Socket {
        id: socket
        path: request.socketPath
        connected: false
        parser: SplitParser {
            onRead: line => {
                if (!request.finished)
                    request.received(line);
            }
        }
        onConnectionStateChanged: {
            if (request.finished)
                return;
            if (connected && !request.submitted) {
                // Mark uncertain before writing: a lost reply never permits a resend.
                request.submitted = true;
                write(request.encodedRequest);
                request.encodedRequest = "";
                flush();
                // The helper permits a four-minute command; allow its reply to arrive.
                deadline.interval = 255000;
                deadline.restart();
            } else if (!connected && request.submitted) {
                request.fail("request connection closed");
            }
        }
        onError: request.fail("request socket error")
    }
}
