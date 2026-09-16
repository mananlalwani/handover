pragma Singleton

import QtQml
import Quickshell
import Quickshell.Io

Singleton {
    id: root

    readonly property int protocolVersion: 1
    readonly property string socketPath: String(Quickshell.env("XDG_RUNTIME_DIR") || "")
        + "/handover/handoverd.sock"
    readonly property bool connected: socketLoader.item
        ? socketLoader.item.connected
        : false
    property var devices: []
    property var notifications: []
    property var lastReceivedShare: null
    property string lastError: ""
    property var pendingCommand: null
    signal commandFinished(string method, var notificationId, bool success, string error)
    signal shareFinished(string method, string deviceId, bool success, string error)

    function sendRequest(method, fields, socketOverride) {
        const socket = socketOverride || socketLoader.item;
        if (!socket || !socket.connected)
            return false;
        socket.write(JSON.stringify(Object.assign({ protocol: protocolVersion, method: method }, fields || {})) + "\n");
        socket.flush();
        return true;
    }

    function sendCommand(method, fields) {
        if (pendingCommand) {
            lastError = "another notification command is still pending";
            return false;
        }
        if (!sendRequest(method, fields)) {
            lastError = "handoverd is disconnected";
            return false;
        }
        pendingCommand = { method: method, notificationId: fields.notification_id };
        lastError = "";
        return true;
    }

    function dismiss(notification) {
        return sendCommand("notification.dismiss", { notification_id: notification.id });
    }

    function invokeAction(notification, action) {
        return sendCommand("notification.action", {
            notification_id: notification.id,
            action_id: action.id
        });
    }

    function reply(notification, text) {
        return sendCommand("notification.reply", {
            notification_id: notification.id,
            text: text
        });
    }

    function sendShare(method, device, fields) {
        if (pendingCommand) {
            lastError = "another command is still pending";
            return false;
        }
        if (!sendRequest(method, Object.assign({ device_id: device.id }, fields))) {
            lastError = "handoverd is disconnected";
            return false;
        }
        pendingCommand = { method: method, deviceId: device.id };
        lastError = "";
        return true;
    }

    function sendUrl(device, url) {
        return sendShare("share.url", device, { url: url });
    }

    function sendFile(device, fileUrl) {
        return sendShare("share.file", device, { file_url: fileUrl });
    }

    function onConnected(socket) {
        retryTimer.stop();
        lastError = "";
        sendRequest("hello", null, socket);
        sendRequest("devices.list", null, socket);
        sendRequest("subscribe", { shares: true }, socket);
    }

    function scheduleReconnect(message) {
        lastError = message;
        devices = [];
        notifications = [];
        lastReceivedShare = null;
        if (pendingCommand) {
            finishPending(false, message);
            pendingCommand = null;
        }
        retryTimer.restart();
    }

    function replaceDevice(device) {
        const next = devices.filter(existing => existing.id !== device.id);
        next.push(device);
        devices = next;
    }

    function removeDevice(deviceId) {
        devices = devices.filter(device => device.id !== deviceId);
    }

    function sameNotificationId(first, second) {
        return first.device_id === second.device_id && first.local_id === second.local_id;
    }

    function replaceNotification(notification) {
        const next = notifications.filter(existing => !sameNotificationId(existing.id, notification.id));
        next.push(notification);
        notifications = next;
    }

    function removeNotification(notificationId) {
        notifications = notifications.filter(notification => !sameNotificationId(notification.id, notificationId));
    }

    function finishPending(success, error) {
        if (!pendingCommand)
            return;
        if (pendingCommand.deviceId)
            shareFinished(pendingCommand.method, pendingCommand.deviceId, success, error);
        else
            commandFinished(pendingCommand.method, pendingCommand.notificationId, success, error);
        pendingCommand = null;
    }

    function handleLine(line) {
        let message;
        try {
            message = JSON.parse(line);
        } catch (error) {
            console.warn("Handover: invalid JSON from daemon:", error);
            return;
        }

        if (message.protocol !== protocolVersion) {
            scheduleReconnect("unsupported daemon protocol " + message.protocol);
            return;
        }

        switch (message.type) {
        case "hello":
            if (!message.supported_protocols.includes(protocolVersion))
                scheduleReconnect("daemon does not support protocol " + protocolVersion);
            break;
        case "devices":
            devices = message.devices || [];
            break;
        case "notifications":
            notifications = message.notifications || [];
            break;
        case "subscribed":
        case "snapshot":
            devices = message.devices || [];
            notifications = message.notifications || [];
            break;
        case "device_added":
        case "device_updated":
            replaceDevice(message.device);
            break;
        case "device_removed":
            removeDevice(message.device_id);
            break;
        case "notification_added":
        case "notification_updated":
            replaceNotification(message.notification);
            break;
        case "notification_removed":
            removeNotification(message.notification_id);
            break;
        case "share_received":
            lastReceivedShare = message.share;
            break;
        case "share_accepted":
            if (pendingCommand && pendingCommand.deviceId === message.device_id)
                finishPending(true, "");
            break;
        case "command_completed":
            if (pendingCommand && pendingCommand.notificationId)
                finishPending(true, "");
            break;
        case "error":
            lastError = message.code + ": " + message.message;
            console.warn("Handover:", lastError);
            finishPending(false, lastError);
            break;
        default:
            console.warn("Handover: unknown daemon message type", message.type);
        }
    }

    Timer {
        id: retryTimer
        interval: 2000
        repeat: false
        onTriggered: {
            socketLoader.active = false;
            recreateTimer.restart();
        }
    }

    Timer {
        id: recreateTimer
        interval: 1
        repeat: false
        onTriggered: socketLoader.active = true
    }

    LazyLoader {
        id: socketLoader
        active: root.socketPath.length > "/handover/handoverd.sock".length

        component: Component {
            Socket {
                id: socket
                property bool wasConnected: false

                path: root.socketPath
                connected: true
                parser: SplitParser {
                    onRead: data => root.handleLine(data)
                }

                onConnectionStateChanged: {
                    if (connected) {
                        wasConnected = true;
                        root.onConnected(socket);
                    } else if (wasConnected) {
                        root.scheduleReconnect("daemon disconnected");
                    }
                }
                onError: error => root.scheduleReconnect("socket error " + error)
            }
        }
    }
}
