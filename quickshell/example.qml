import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import "components"
import "pages"

ShellRoot {
    FloatingWindow {
        id: window

        property string page: "overview"
        readonly property var navigation: [{
            "key": "overview",
            "label": "Overview",
            "group": "Workspace",
            "description": "Your phones and connection"
        }, {
            "key": "messages",
            "label": "Messages",
            "group": "",
            "description": "Conversations and accounts"
        }, {
            "key": "calls",
            "label": "Calls",
            "group": "",
            "description": "Call controls and audio status"
        }, {
            "key": "notifications",
            "label": "Notifications",
            "group": "",
            "description": "Notifications from your phones"
        }, {
            "key": "media",
            "label": "Media",
            "group": "",
            "description": "Laptop audio and phone playback"
        }, {
            "key": "transfers",
            "label": "Send & transfers",
            "group": "Sharing",
            "description": "Send a file or link to your phone"
        }, {
            "key": "clipboard",
            "label": "Clipboard",
            "group": "",
            "description": "Send text and manage clipboard history"
        }, {
            "key": "files",
            "label": "Phone files",
            "group": "",
            "description": "Browse files shared by your phone"
        }, {
            "key": "contacts",
            "label": "Contacts",
            "group": "Tools",
            "description": "Contacts synced from your phone"
        }, {
            "key": "commands",
            "label": "Commands",
            "group": "",
            "description": "Run configured desktop commands"
        }, {
            "key": "remote-input",
            "label": "Remote input",
            "group": "",
            "description": "Send input to a connected phone"
        }]
        readonly property var currentPage: navigation.find((item) => {
            return item.key === page;
        }) || navigation[0]
        // Keep the development emulator out of the everyday device picker.
        readonly property var appDevices: HandoverService.devices.filter((device) => {
            return !device.name.includes("sdk_gphone");
        })
        property string selectedDeviceId: ""
        readonly property var selectedDevice: appDevices.find((device) => {
            return device.id === selectedDeviceId;
        }) || null
        readonly property bool selectedDeviceAvailable: !!selectedDevice && selectedDevice.connected && selectedDevice.paired

        visible: true
        implicitWidth: 1120
        implicitHeight: 780
        minimumSize: Qt.size(840, 600)
        color: "#111720"
        title: "Handover"
        onAppDevicesChanged: {
            if (!appDevices.some((device) => {
                return device.id === selectedDeviceId;
            }))
                selectedDeviceId = (appDevices.find((device) => {
                return device.connected && device.paired;
            }) || appDevices[0] || {
            }).id || "";

        }

        Pane {
            anchors.fill: parent
            padding: 0
            font.pixelSize: 13
            palette.window: "#111720"
            palette.windowText: "#edf1f7"
            palette.base: "#131b26"
            palette.alternateBase: "#202a38"
            palette.text: "#edf1f7"
            palette.button: "#263344"
            palette.buttonText: "#edf1f7"
            palette.highlight: "#355c88"
            palette.highlightedText: "#ffffff"
            palette.mid: "#354154"
            palette.dark: "#111720"

            RowLayout {
                anchors.fill: parent
                spacing: 0

                Rectangle {
                    Layout.preferredWidth: window.width < 1000 ? 176 : 204
                    Layout.fillHeight: true
                    color: "#151d28"

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: 16
                        spacing: 12

                        Text {
                            Layout.topMargin: 8
                            Layout.bottomMargin: 8
                            text: "handover"
                            color: "#edf1f7"
                            font.pixelSize: 25
                            font.weight: Font.DemiBold
                            font.letterSpacing: -0.8
                        }

                        ScrollView {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            contentWidth: availableWidth
                            clip: true
                            ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

                            ColumnLayout {
                                width: parent.width
                                spacing: 4

                                Repeater {
                                    model: window.navigation

                                    delegate: ColumnLayout {
                                        required property var modelData

                                        Layout.fillWidth: true
                                        spacing: 6

                                        Text {
                                            Layout.topMargin: 8
                                            Layout.leftMargin: 12
                                            Layout.bottomMargin: 4
                                            visible: modelData.group.length > 0
                                            text: modelData.group.toUpperCase()
                                            color: "#8291a6"
                                            font.pixelSize: 10
                                            font.letterSpacing: 1.2
                                            font.weight: Font.DemiBold
                                        }

                                        Button {
                                            id: navButton

                                            Layout.fillWidth: true
                                            implicitHeight: 36
                                            text: modelData.label
                                            hoverEnabled: true
                                            Accessible.name: text
                                            Accessible.role: Accessible.PageTab
                                            Accessible.selected: window.page === modelData.key
                                            onClicked: window.page = modelData.key

                                            background: Rectangle {
                                                radius: 8
                                                color: window.page === modelData.key ? "#2b405b" : navButton.hovered ? "#202c3c" : "transparent"
                                                border.color: navButton.activeFocus ? "#9bc5f4" : "transparent"
                                            }

                                            contentItem: Text {
                                                text: navButton.text
                                                color: window.page === modelData.key ? "#ffffff" : "#aab7c8"
                                                font.pixelSize: 13
                                                font.weight: window.page === modelData.key ? Font.DemiBold : Font.Normal
                                                leftPadding: 12
                                                verticalAlignment: Text.AlignVCenter
                                                elide: Text.ElideRight
                                            }

                                        }

                                    }

                                }

                            }

                        }

                        RowLayout {
                            Layout.topMargin: 10
                            spacing: 8

                            Rectangle {
                                width: 7
                                height: 7
                                radius: 4
                                color: HandoverService.connected ? "#91c9a6" : "#e7ad83"
                            }

                            Text {
                                text: HandoverService.connected ? "Service connected" : "Service offline"
                                color: "#aab7c8"
                                font.pixelSize: 11
                            }

                        }

                    }

                }

                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.margins: window.width < 1000 ? 20 : 28
                    spacing: 18

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 16

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 5

                            Text {
                                Layout.fillWidth: true
                                text: window.currentPage.label
                                color: "#edf1f7"
                                font.pixelSize: 26
                                font.weight: Font.DemiBold
                                elide: Text.ElideRight
                            }

                            Text {
                                Layout.fillWidth: true
                                text: window.currentPage.description
                                color: "#91a1b7"
                                font.pixelSize: 13
                                wrapMode: Text.WordWrap
                            }

                        }

                        ColumnLayout {
                            visible: ["overview", "calls", "transfers", "clipboard", "files", "contacts", "remote-input"].includes(window.page)
                            Layout.preferredWidth: Math.min(240, mainContent.width * 0.36)
                            Layout.maximumWidth: 240
                            Layout.minimumWidth: 0
                            spacing: 4

                            ChoiceBox {
                                Layout.fillWidth: true
                                model: window.appDevices
                                textRole: "name"
                                currentIndex: window.appDevices.findIndex((device) => {
                                    return device.id === window.selectedDeviceId;
                                })
                                displayText: window.appDevices.length === 0 ? "No phone paired" : currentText
                                enabled: window.appDevices.length > 0
                                onActivated: (index) => {
                                    return window.selectedDeviceId = window.appDevices[index].id;
                                }
                                Accessible.name: "Selected phone"
                            }

                            Text {
                                Layout.fillWidth: true
                                text: !window.selectedDevice ? "Pair a phone to get started" : (window.selectedDevice.connected ? "Connected" : "Offline") + (window.selectedDevice.battery ? " · " + window.selectedDevice.battery.percentage + "% battery" : "")
                                color: window.selectedDeviceAvailable ? "#91c9a6" : "#91a1b7"
                                font.pixelSize: 11
                                elide: Text.ElideRight
                            }

                        }

                    }

                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: offlineNotice.implicitHeight + 24
                        radius: 10
                        color: "#2d2925"
                        visible: !HandoverService.connected

                        Text {
                            id: offlineNotice

                            anchors.fill: parent
                            anchors.margins: 12
                            text: "Handover is reconnecting to the desktop service. Actions will be available when it returns."
                            color: "#e8c29d"
                            font.pixelSize: 13
                            wrapMode: Text.WordWrap
                        }

                    }

                    Item {
                        id: mainContent

                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true

                        ScrollView {
                            anchors.fill: parent
                            visible: window.page === "overview"
                            contentWidth: availableWidth
                            clip: true
                            ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

                            OverviewDevicesPage {
                                window: window
                                width: parent.width
                            }

                        }

                        ColumnLayout {
                            anchors.fill: parent
                            visible: window.page === "messages"
                            spacing: 12

                            MessagesAppPrompt {
                                id: messagesApp

                                Layout.fillWidth: true
                                Layout.fillHeight: !showFallback
                            }

                            MessagesPage {
                                id: messagesCard

                                window: window
                                Layout.fillWidth: true
                                Layout.fillHeight: true
                                visible: window.page === "messages" && messagesApp.showFallback
                            }

                        }

                        CallsPage {
                            window: window
                            anchors.fill: parent
                            visible: window.page === "calls"
                        }

                        MediaPage {
                            window: window
                            anchors.fill: parent
                        }

                        NotificationsPage {
                            window: window
                            anchors.fill: parent
                        }

                        ContactsPage {
                            window: window
                            anchors.fill: parent
                        }

                        ClipboardPage {
                            window: window
                            anchors.fill: parent
                        }

                        FilesPage {
                            window: window
                            anchors.fill: parent
                        }

                        CommandsPage {
                            window: window
                            anchors.fill: parent
                        }

                        RemoteInputPage {
                            window: window
                            anchors.fill: parent
                        }

                        TransfersPage {
                            window: window
                            anchors.fill: parent
                        }

                    }

                }

            }

            background: Rectangle {
                color: "#111720"
            }

        }

        Connections {
            function onHistoryProgress(conversationId, complete, messageCount) {
                if (messagesCard.selectedConversation && HandoverService.sameConversationId(conversationId, messagesCard.selectedConversation))
                    messagesCard.status = complete ? "Complete history loaded" : "Loading older messages…";

            }

            function onMessagingFinished(method, requestId, success, error) {
                messagesCard.status = success ? "Request accepted" : error;
            }

            function onMessagingAccepted(method, subject, success, error) {
                messagesCard.status = success ? "Request accepted" : error;
            }

            target: HandoverService
        }

    }

}
