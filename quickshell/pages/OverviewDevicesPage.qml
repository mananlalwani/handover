import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import ".."

Rectangle {
    required property var window
    id: deviceCard
    Layout.fillWidth: true
    implicitHeight: deviceColumn.implicitHeight + 24
    radius: 12
    color: "#252d3b"
    visible: window.page === "overview"

    Dialog {
        id: forgetDeviceDialog
        property var deviceToForget: null
        title: "Forget native phone?"
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        Label {
            text: forgetDeviceDialog.deviceToForget
                ? "Remove trust for " + forgetDeviceDialog.deviceToForget.name + "?"
                : "Remove native phone trust?"
            wrapMode: Text.WordWrap
        }
        onAccepted: {
            if (deviceToForget)
                HandoverService.nativeAction(deviceToForget, "unpair");
            deviceToForget = null;
        }
        onRejected: deviceToForget = null
    }

    ColumnLayout {
        id: deviceColumn
        anchors.fill: parent
        anchors.margins: 12
        spacing: 6

        Text {
            Layout.fillWidth: true
            text: "Native phones"
            color: "#ffffff"
            font.pixelSize: 16
            font.bold: true
        }

        Repeater {
            model: window.appDevices
            delegate: ColumnLayout {
                required property var modelData
                Layout.fillWidth: true
                spacing: 3

                Text {
                    Layout.fillWidth: true
                    text: modelData.name + " · "
                        + (modelData.connected ? "connected" : "offline")
                        + (modelData.paired ? " · paired" : " · not paired")
                    color: "#f3f4f6"
                }

                Text {
                    Layout.fillWidth: true
                    text: {
                        const connectivity = modelData.connectivity;
                        if (!connectivity)
                            return "Network: unavailable";
                        return "Network: " + connectivity.transport
                            + (connectivity.metered ? " · metered" : " · unmetered")
                            + (connectivity.validated ? " · validated" : " · unvalidated");
                    }
                    color: "#b9c2cf"
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 6

                    Button {
                        text: "Ping"
                        enabled: modelData.connected && modelData.paired
                            && !HandoverService.nativePending
                        onClicked: HandoverService.nativeAction(modelData, "ping")
                    }
                    Button {
                        text: "Ring"
                        enabled: modelData.connected && modelData.paired
                            && !HandoverService.nativePending
                        onClicked: HandoverService.nativeAction(modelData, "ring")
                    }
                    Button {
                        text: "Lock"
                        enabled: modelData.connected && modelData.paired
                            && !HandoverService.nativePending
                        onClicked: HandoverService.nativeAction(modelData, "lock")
                    }
                    Button {
                        text: "Keep awake"
                        enabled: modelData.connected && modelData.paired
                        onClicked: HandoverService.setPhoneAwake(modelData, true)
                    }
                    Button {
                        text: "Tethering"
                        enabled: modelData.connected && modelData.paired
                        onClicked: HandoverService.openPhoneTethering(modelData)
                    }
                    Button {
                        text: "Forget"
                        enabled: modelData.paired && !HandoverService.nativePending
                        onClicked: {
                            forgetDeviceDialog.deviceToForget = modelData;
                            forgetDeviceDialog.open();
                             }
                         }
                }
            }
        }

        Text {
            Layout.fillWidth: true
            text: HandoverService.nativeNotice
            color: HandoverService.nativeNotice.indexOf("accepted") >= 0
                || HandoverService.nativeNotice.startsWith("Phone action")
                ? "#a7e3b5" : "#ffb4ab"
            visible: text.length > 0
        }

        Repeater {
            model: HandoverService.pendingPeers
            delegate: RowLayout {
                required property var modelData
                Layout.fillWidth: true
                Text {
                    Layout.fillWidth: true
                    text: "Pair " + modelData.name + " · " + modelData.code
                    color: "#f3f4f6"
                }
                Button {
                    text: "Approve"
                    onClicked: HandoverService.approvePair(modelData.id, modelData.code)
                }
            }
        }

        Text {
            Layout.fillWidth: true
            visible: !!HandoverService.lastReceivedShare
            color: "#cde4ff"
            wrapMode: Text.WordWrap
            text: HandoverService.lastReceivedShare
                ? "Incoming share: " + (HandoverService.lastReceivedShare.resource.kind === "url"
                    ? HandoverService.lastReceivedShare.resource.url
                    : HandoverService.lastReceivedShare.resource.path)
                : ""
        }

        Text {
            Layout.fillWidth: true
            visible: !!HandoverService.lastShareProgress
            color: "#b9c2cf"
            text: HandoverService.lastShareProgress
                ? "Transfer " + HandoverService.lastShareProgress.transfer_id
                    + " · " + HandoverService.lastShareProgress.bytes_sent
                    + " / " + HandoverService.lastShareProgress.total_bytes
                : ""
        }

        Text {
            Layout.fillWidth: true
            text: "Screensaver inhibition is automatic while a native phone is connected."
            color: "#b9c2cf"
            wrapMode: Text.WordWrap
        }
        Text {
            Layout.fillWidth: true
            text: "Desktop media pauses automatically when a phone call starts."
            color: "#b9c2cf"
            wrapMode: Text.WordWrap
        }
    }
}
