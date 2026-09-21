import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell

Rectangle {
    required property var window
    id: notificationCard
    Layout.fillWidth: true
    Layout.fillHeight: true
    radius: 12
    color: "#252d3b"
    visible: window.page === "overview" || window.page === "notifications"
    property var selected: HandoverService.notifications.length > 0
        ? HandoverService.notifications[0] : null

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 12
        spacing: 8
        Text {
            text: "Notifications"
            color: "#ffffff"
            font.pixelSize: 16
            font.bold: true
        }
        ListView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: HandoverService.notifications
            delegate: ColumnLayout {
                required property var modelData
                property var note: modelData
                width: ListView.view.width
                spacing: 4
                Text {
                    Layout.fillWidth: true
                    color: "#9fb4cc"
                    text: modelData.app_name
                }
                Text {
                    Layout.fillWidth: true
                    color: "#ffffff"
                    font.bold: true
                    wrapMode: Text.Wrap
                    text: modelData.title
                }
                Text {
                    Layout.fillWidth: true
                    color: "#d5dde8"
                    wrapMode: Text.Wrap
                    text: modelData.body || ""
                }
                RowLayout {
                    Layout.fillWidth: true
                    TextField {
                        id: itemReply
                        Layout.fillWidth: true
                        visible: modelData.reply_supported
                        placeholderText: "Reply"
                        onAccepted: {
                            if (text.trim().length > 0)
                                HandoverService.reply(modelData, text);
                        }
                    }
                    Button {
                        text: "Send"
                        visible: modelData.reply_supported
                        onClicked: {
                            if (itemReply.text.trim().length > 0)
                                HandoverService.reply(modelData, itemReply.text);
                        }
                    }
                    Button {
                        text: "Dismiss"
                        visible: modelData.clearable
                        onClicked: HandoverService.dismiss(modelData)
                    }
                }
                Row {
                    spacing: 6
                    Repeater {
                        model: modelData.actions || []
                        Button {
                            required property var modelData
                            text: modelData.label
                            onClicked: HandoverService.invokeAction(note, modelData)
                        }
                    }
                }
            }
        }
        Text {
            visible: HandoverService.notifications.length === 0
            color: "#b9c2cf"
            text: "No active notifications"
        }
        TextField {
            id: replyField
            visible: false
        }
    }

    Connections {
        target: HandoverService
        function onCommandFinished(method, notificationId, success, error) {
            if (method === "notification.reply" && success)
                replyField.text = "";
        }
    }
}
