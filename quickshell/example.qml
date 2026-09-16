import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import Quickshell

ShellRoot {
    FloatingWindow {
        id: window
        visible: true
        implicitWidth: 420
        implicitHeight: 470
        color: "#20242b"
        title: "Handover"
        property var shareDevices: HandoverService.devices.filter(device =>
            device.connected && device.paired && device.capabilities.includes("file_transfer"))
        property string shareStatus: ""

        FileDialog {
            id: fileDialog
            title: "Send one file to phone"
            fileMode: FileDialog.OpenFile
            onAccepted: {
                const device = window.shareDevices[devicePicker.currentIndex];
                if (device && !HandoverService.sendFile(device, selectedFile.toString()))
                    window.shareStatus = HandoverService.lastError;
            }
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 16
            spacing: 12

            Text {
                Layout.fillWidth: true
                color: "#f4f4f5"
                font.pixelSize: 18
                text: {
                    const connected = HandoverService.devices.filter(device => device.connected);
                    if (connected.length === 0)
                        return "No connected device";
                    const device = connected[0];
                    const battery = device.battery
                        ? device.battery.percentage + "%"
                        : "battery unavailable";
                    return device.name + " · " + battery;
                }
            }

            RowLayout {
                Layout.fillWidth: true

                ComboBox {
                    id: devicePicker
                    Layout.fillWidth: true
                    model: window.shareDevices.map(device => device.name)
                    enabled: window.shareDevices.length > 0
                }

                Button {
                    text: "Send file"
                    enabled: devicePicker.enabled && !HandoverService.pendingCommand
                    onClicked: fileDialog.open()
                }
            }

            RowLayout {
                Layout.fillWidth: true

                TextField {
                    id: urlField
                    Layout.fillWidth: true
                    placeholderText: "URL to send"
                }

                Button {
                    text: "Send URL"
                    enabled: devicePicker.enabled && urlField.text.length > 0
                        && !HandoverService.pendingCommand
                    onClicked: {
                        const device = window.shareDevices[devicePicker.currentIndex];
                        if (device && !HandoverService.sendUrl(device, urlField.text))
                            window.shareStatus = HandoverService.lastError;
                    }
                }
            }

            Text {
                Layout.fillWidth: true
                color: "#b9c2cf"
                textFormat: Text.PlainText
                text: window.shareStatus
                visible: text.length > 0
            }

            Rectangle {
                id: notificationCard
                Layout.fillWidth: true
                Layout.fillHeight: true
                radius: 8
                color: "#303741"
                property var notification: HandoverService.notifications.find(item => item.reply_supported)
                    || (HandoverService.notifications.length > 0
                        ? HandoverService.notifications[0] : null)

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 12
                    spacing: 6

                    Text {
                        Layout.fillWidth: true
                        color: "#b9c2cf"
                        textFormat: Text.PlainText
                        text: {
                            const notification = notificationCard.notification;
                            if (!notification)
                                return "Phone notifications";
                            const device = HandoverService.devices.find(item =>
                                item.id === notification.id.device_id);
                            return notification.app_name + " · "
                                + (device ? device.name : notification.id.device_id);
                        }
                    }

                    Text {
                        Layout.fillWidth: true
                        color: "#ffffff"
                        font.pixelSize: 17
                        textFormat: Text.PlainText
                        elide: Text.ElideRight
                        text: notificationCard.notification
                            ? notificationCard.notification.title : "No active notifications"
                    }

                    Text {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        color: "#e4e7eb"
                        textFormat: Text.PlainText
                        wrapMode: Text.Wrap
                        elide: Text.ElideRight
                        text: notificationCard.notification
                            ? notificationCard.notification.body : ""
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        visible: notificationCard.notification !== null
                        spacing: 6

                        TextField {
                            id: replyField
                            Layout.fillWidth: true
                            visible: notificationCard.notification
                                && notificationCard.notification.reply_supported
                            placeholderText: "Reply from desktop"
                            enabled: !HandoverService.pendingCommand
                            onAccepted: sendReply()

                            function sendReply() {
                                if (notificationCard.notification && text.trim().length > 0)
                                    HandoverService.reply(notificationCard.notification, text);
                            }
                        }

                        Button {
                            text: "Send"
                            visible: replyField.visible
                            enabled: !HandoverService.pendingCommand && replyField.text.trim().length > 0
                            onClicked: replyField.sendReply()
                        }

                        Button {
                            text: "Dismiss"
                            visible: notificationCard.notification
                                && notificationCard.notification.clearable
                            enabled: !HandoverService.pendingCommand
                            onClicked: HandoverService.dismiss(notificationCard.notification)
                        }
                    }

                    Row {
                        visible: notificationCard.notification
                            && notificationCard.notification.actions.length > 0
                        spacing: 6
                        Repeater {
                            model: notificationCard.notification
                                ? notificationCard.notification.actions : []
                            Button {
                                required property var modelData
                                text: modelData.label
                                enabled: !HandoverService.pendingCommand
                                onClicked: HandoverService.invokeAction(
                                    notificationCard.notification, modelData)
                            }
                        }
                    }

                    Text {
                        Layout.fillWidth: true
                        color: "#ffb4ab"
                        textFormat: Text.PlainText
                        wrapMode: Text.Wrap
                        visible: HandoverService.lastError.length > 0
                        text: HandoverService.lastError
                    }
                }
            }
        }

        Connections {
            target: HandoverService
            function onCommandFinished(method, notificationId, success, error) {
                if (method === "notification.reply" && success)
                    replyField.text = "";
            }
            function onShareFinished(method, deviceId, success, error) {
                window.shareStatus = success
                    ? "Accepted by KDE Connect; delivery is not confirmed"
                    : error;
            }
        }
    }
}
