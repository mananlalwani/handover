import ".."
import "../components"
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Rectangle {
    id: deviceCard

    required property var window

    implicitHeight: deviceColumn.implicitHeight + 40
    radius: 12
    color: "#1b2532"
    visible: window.page === "overview"

    Dialog {
        id: forgetDeviceDialog

        property var deviceToForget: null

        anchors.centerIn: parent
        title: "Forget phone?"
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        onAccepted: {
            if (deviceToForget)
                HandoverService.nativeAction(deviceToForget, "unpair");

            deviceToForget = null;
        }
        onRejected: deviceToForget = null

        Label {
            text: forgetDeviceDialog.deviceToForget ? "Remove trust for " + forgetDeviceDialog.deviceToForget.name + "?" : "Remove phone trust?"
            textFormat: Text.PlainText
            wrapMode: Text.WordWrap
        }

    }

    ColumnLayout {
        id: deviceColumn

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 20
        spacing: 18

        Text {
            text: "Your devices"
            color: "#edf1f7"
            font.pixelSize: 17
            font.weight: Font.DemiBold
        }

        EmptyState {
            Layout.fillWidth: true
            visible: window.appDevices.length === 0
            title: "No phones yet"
            description: "Open Handover on your phone and choose Connect / Pair desktop. Keep both devices reachable on your local network or VPN."
        }

        Repeater {
            model: window.appDevices

            delegate: ColumnLayout {
                required property var modelData

                Layout.fillWidth: true
                spacing: 10

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 10

                    Rectangle {
                        width: 9
                        height: 9
                        radius: 5
                        color: modelData.connected ? "#91c9a6" : "#8291a6"
                    }

                    Text {
                        Layout.fillWidth: true
                        text: modelData.name
                        textFormat: Text.PlainText
                        elide: Text.ElideRight
                        color: "#edf1f7"
                        font.pixelSize: 16
                        font.weight: Font.DemiBold
                    }

                    Text {
                        text: (modelData.connected ? "Connected" : "Offline") + (modelData.battery ? " · " + modelData.battery.percentage + "%" : "")
                        color: modelData.connected ? "#91c9a6" : "#aab7c8"
                        font.pixelSize: 12
                    }

                }

                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: {
                        const network = modelData.connectivity;
                        if (!modelData.paired)
                            return "Waiting for pairing approval";

                        if (!modelData.connected)
                            return "Open Handover on the phone to reconnect.";

                        return network ? "Phone network: " + network.transport + (network.metered ? " · metered" : "") : "Phone network not reported";
                    }
                    color: "#aab7c8"
                    font.pixelSize: 13
                }

                Flow {
                    Layout.fillWidth: true
                    spacing: 8

                    ActionButton {
                        text: "Ping"
                        enabled: modelData.connected && modelData.paired && !HandoverService.nativePending
                        onClicked: HandoverService.nativeAction(modelData, "ping")
                    }

                    ActionButton {
                        text: "Ring phone"
                        enabled: modelData.connected && modelData.paired && !HandoverService.nativePending
                        onClicked: HandoverService.nativeAction(modelData, "ring")
                    }

                    ActionButton {
                        text: "Lock"
                        enabled: modelData.connected && modelData.paired && !HandoverService.nativePending
                        onClicked: HandoverService.nativeAction(modelData, "lock")
                    }

                    ActionButton {
                        text: "Keep awake"
                        enabled: modelData.connected && modelData.paired
                        onClicked: HandoverService.setPhoneAwake(modelData, true)
                    }

                    ActionButton {
                        text: "Tethering settings"
                        enabled: modelData.connected && modelData.paired
                        onClicked: HandoverService.openPhoneTethering(modelData)
                    }

                    ActionButton {
                        text: "Forget…"
                        danger: true
                        enabled: modelData.paired && !HandoverService.nativePending
                        onClicked: {
                            forgetDeviceDialog.deviceToForget = modelData;
                            forgetDeviceDialog.open();
                        }
                    }

                }

                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: 1
                    color: "#2d394a"
                }

            }

        }

        Text {
            Layout.fillWidth: true
            text: HandoverService.nativeNotice
            textFormat: Text.PlainText
            wrapMode: Text.WordWrap
            color: "#c6d5e8"
            visible: text.length > 0
        }

        Repeater {
            model: HandoverService.pendingPeers

            delegate: ColumnLayout {
                required property var modelData

                Layout.fillWidth: true
                spacing: 8

                Text {
                    Layout.fillWidth: true
                    text: "Pair " + modelData.name + " · " + modelData.code
                    textFormat: Text.PlainText
                    wrapMode: Text.WordWrap
                    color: "#edf1f7"
                }

                ActionButton {
                    text: "Approve pairing"
                    primary: true
                    onClicked: HandoverService.approvePair(modelData.id, modelData.code)
                }

            }

        }

        Text {
            Layout.fillWidth: true
            color: "#8291a6"
            font.pixelSize: 12
            wrapMode: Text.WordWrap
            text: "The desktop stays awake while a phone is connected. Laptop media pauses when a phone call starts."
        }

    }

}
