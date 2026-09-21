import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import Quickshell
import "pages"

ShellRoot {
    FloatingWindow {
        id: window
        visible: true
        implicitWidth: 1080
        implicitHeight: 780
        color: "#10141c"
        title: "Handover"
        property string page: "overview"
        // The reference UI is focused on the native Handover path for now.
        // Keep KDE Connect available in the daemon, but do not mix its
        // compatibility devices (or the development emulator) into this UI.
        property var appDevices: HandoverService.devices.filter(device =>
            device.id.startsWith("native:") && !device.name.includes("sdk_gphone"))
        property var shareDevices: window.appDevices.filter(device =>
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

        RowLayout {
            id: shellLayout
            anchors.fill: parent
            anchors.margins: 14
            spacing: 12

            Rectangle {
                Layout.preferredWidth: 196
                Layout.fillHeight: true
                radius: 16
                color: "#1b2230"
                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 12
                    spacing: 4
                    Text {
                        text: "Handover"
                        color: "#f4f7fb"
                        font.pixelSize: 22
                        font.bold: true
                    }
                    Text {
                        Layout.fillWidth: true
                        wrapMode: Text.WordWrap
                        color: HandoverService.connected ? "#8fd4a8" : "#ffb4ab"
                        text: HandoverService.connected ? "Daemon connected" : "Daemon offline"
                    }
                    Repeater {
                        model: [
                            { key: "overview", label: "Overview" },
                            { key: "messages", label: "Messages" },
                            { key: "calls", label: "Calls" },
                            { key: "notifications", label: "Notifications" },
                            { key: "media", label: "Media" },
                            { key: "contacts", label: "Contacts" },
                            { key: "clipboard", label: "Clipboard" },
                            { key: "files", label: "Phone files" },
                            { key: "commands", label: "Commands" },
                            { key: "remote-input", label: "Remote input" },
                            { key: "transfers", label: "Transfers" }
                        ]
                        delegate: Button {
                            required property var modelData
                            Layout.fillWidth: true
                            text: modelData.label
                            flat: window.page !== modelData.key
                            highlighted: window.page === modelData.key
                            onClicked: window.page = modelData.key
                            background: Rectangle {
                                radius: 10
                                color: window.page === modelData.key ? "#314b6c" : "transparent"
                            }
                            contentItem: Text {
                                text: parent.text
                                color: "#eef2f7"
                                font.pixelSize: 14
                                leftPadding: 8
                                verticalAlignment: Text.AlignVCenter
                            }
                        }
                    }
                    Item { Layout.fillHeight: true }
                }
            }

            ColumnLayout {
            id: mainLayout
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 12

            Text {
                Layout.fillWidth: true
                color: "#f4f4f5"
                    font.pixelSize: 20
                    font.bold: true
                    text: {
                    const connected = window.appDevices.filter(device => device.connected);
                    if (connected.length === 0)
                        return "No connected device";
                    const device = connected[0];
                    const battery = device.battery
                        ? device.battery.percentage + "%"
                        : "battery unavailable";
                    return device.name + " · " + battery;
                }
            }

            OverviewDevicesPage {
                window: window
                Layout.fillWidth: true
            }

            RemoteInputPage {
                window: window
                Layout.fillWidth: true
            }

            Item {
                id: messageContentArea
                Layout.fillWidth: true
                Layout.fillHeight: true
                visible: window.page === "messages"
            }

            Item {
                id: callContentArea
                Layout.fillWidth: true
                Layout.fillHeight: true
                visible: window.page === "calls"
            }

            MediaPage {
                window: window
                Layout.fillWidth: true
            }

            ContactsPage {
                window: window
                Layout.fillWidth: true
                Layout.fillHeight: true
            }

            ClipboardPage {
                window: window
                Layout.fillWidth: true
                Layout.fillHeight: true
            }

            FilesPage {
                window: window
                Layout.fillWidth: true
                Layout.fillHeight: true
            }

            CommandsPage {
                window: window
                Layout.fillWidth: true
                Layout.fillHeight: true
            }

            RowLayout {
                Layout.fillWidth: true
                visible: window.page === "overview" || window.page === "transfers"

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
                visible: window.page === "overview" || window.page === "transfers"

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
                visible: text.length > 0 && (window.page === "overview"
                    || window.page === "transfers")
            }

            NotificationsPage {
                window: window
                Layout.fillWidth: true
                Layout.fillHeight: true
            }
        }
        }

        MessagesPage {
            id: messagesCard
            window: window
            parent: messageContentArea
            anchors.fill: parent
        }
        CallsPage {
            window: window
            parent: callContentArea
            anchors.fill: parent
        }

        Connections {
            target: HandoverService
            function onHistoryProgress(conversationId, complete, messageCount) {
                if (messagesCard.selectedConversation
                    && HandoverService.sameConversationId(
                        conversationId, messagesCard.selectedConversation))
                    messagesCard.status = complete
                        ? "complete history loaded"
                        : "loading older messages…";
            }
            function onShareFinished(method, deviceId, success, error) {
                window.shareStatus = success
                    ? "Share accepted by the daemon; delivery is not confirmed"
                    : error;
            }
            function onMessagingFinished(method, requestId, success, error) {
                messagesCard.status = success ? "accepted: " + requestId : error;
            }
            function onMessagingAccepted(method, subject, success, error) {
                messagesCard.status = success ? "accepted" : error;
            }
        }
    }
}
