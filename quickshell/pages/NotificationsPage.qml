import ".."
import "../components"
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell

Rectangle {
    id: notificationCard

    required property var window
    property var selected: HandoverService.notifications.length > 0 ? HandoverService.notifications[0] : null

    Layout.fillWidth: true
    Layout.fillHeight: true
    radius: 12
    color: "#1b2532"
    visible: window.page === "notifications"

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 20
        spacing: 8

        ListView {
            spacing: 14
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: HandoverService.notifications

            ScrollBar.vertical: ScrollBar {
            }

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
                    textFormat: Text.PlainText
                    text: modelData.title
                }

                Text {
                    Layout.fillWidth: true
                    color: "#d5dde8"
                    wrapMode: Text.Wrap
                    textFormat: Text.PlainText
                    text: modelData.body || ""
                }

                RowLayout {
                    Layout.fillWidth: true

                    InputField {
                        id: itemReply

                        Layout.fillWidth: true
                        visible: modelData.reply_supported
                        placeholderText: "Reply"
                        onAccepted: {
                            if (text.trim().length > 0)
                                HandoverService.reply(modelData, text);

                        }
                    }

                    ActionButton {
                        text: "Send"
                        visible: modelData.reply_supported
                        onClicked: {
                            if (itemReply.text.trim().length > 0)
                                HandoverService.reply(modelData, itemReply.text);

                        }
                    }

                    ActionButton {
                        text: "Dismiss"
                        visible: modelData.clearable
                        onClicked: HandoverService.dismiss(modelData)
                    }

                }

                Flow {
                    Layout.fillWidth: true
                    spacing: 6

                    Repeater {
                        model: modelData.actions || []

                        ActionButton {
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

        InputField {
            id: replyField

            visible: false
        }

    }

    Connections {
        function onCommandFinished(method, notificationId, success, error) {
            if (method === "notification.reply" && success)
                replyField.text = "";

        }

        target: HandoverService
    }

}
