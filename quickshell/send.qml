import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import Quickshell
import "Share.js" as Share

ShellRoot {
    FloatingWindow {
        id: window
        visible: true
        title: "Send to phone"
        implicitWidth: 500
        implicitHeight: 360
        color: "#10141c"
        property string resource: String(Quickshell.env("HANDOVER_SHARE_RESOURCE") || "")
        property string deviceId: ""
        property var choices: Share.devices(HandoverService.devices)
        property var transfer: null
        readonly property bool locked: !!transfer && transfer.phase !== "done"

        function submit() {
            const device = choices[picker.currentIndex];
            const item = Share.resource(resource);
            if (!device || !item)
                return;
            transfer = Share.start(device.id, item.method);
            const sent = item.method === "share.file"
                ? HandoverService.sendFile(device, item.value)
                : HandoverService.sendUrl(device, item.value);
            if (!sent)
                transfer = Share.failed(transfer, HandoverService.lastError);
        }

        FileDialog {
            id: fileDialog
            title: "Choose a file to send"
            fileMode: FileDialog.OpenFile
            onAccepted: window.resource = selectedFile.toString()
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 20
            spacing: 12
            Label {
                text: "Send to phone"
                color: "#ffffff"
                font.pixelSize: 22
                font.bold: true
            }
            Label {
                Layout.fillWidth: true
                text: !HandoverService.connected ? "Waiting for the Handover daemon…"
                    : window.choices.length === 0 ? "No connected device supports sharing." : "Choose a device"
                color: "#b9c2cf"
                wrapMode: Text.WordWrap
            }
            ComboBox {
                id: picker
                Layout.fillWidth: true
                model: window.choices.map(device => device.name + " · " + device.id)
                currentIndex: window.deviceId ? window.choices.findIndex(device => device.id === window.deviceId)
                    : window.choices.length > 0 ? 0 : -1
                enabled: !window.locked
                onActivated: index => window.deviceId = window.choices[index].id
            }
            RowLayout {
                TextField {
                    Layout.fillWidth: true
                    text: window.resource
                    placeholderText: "Paste an http or https link"
                    enabled: !window.locked
                    onTextEdited: window.resource = text
                    selectByMouse: true
                }
                Button {
                    text: "Choose file"
                    enabled: !window.locked
                    onClicked: fileDialog.open()
                }
            }
            Label {
                Layout.fillWidth: true
                text: window.transfer ? window.transfer.text : "Choose one file or paste a link."
                textFormat: Text.PlainText
                wrapMode: Text.WrapAnywhere
                color: "#b9c2cf"
            }
            Item { Layout.fillHeight: true }
            RowLayout {
                Button {
                    text: "Send"
                    enabled: HandoverService.connected && picker.currentIndex >= 0
                        && !!Share.resource(window.resource) && !window.locked
                        && !HandoverService.pendingCommand
                    onClicked: window.submit()
                }
                Button {
                    text: "New share"
                    visible: !!window.transfer && window.transfer.phase !== "pending"
                    onClicked: window.transfer = null
                }
                Item { Layout.fillWidth: true }
                Button {
                    text: "Close"
                    onClicked: Qt.quit()
                }
            }
            Label {
                Layout.fillWidth: true
                text: "Closing this window does not cancel an accepted transfer."
                wrapMode: Text.WordWrap
                color: "#b9c2cf"
                font.pixelSize: 12
            }
        }

        Connections {
            target: HandoverService
            function onShareAccepted(method, deviceId, transferId) {
                window.transfer = Share.accepted(window.transfer, method, deviceId, transferId);
            }
            function onShareFinished(method, deviceId, success, error) {
                if (!success && window.transfer && window.transfer.deviceId === deviceId
                        && window.transfer.method === method)
                    window.transfer = Share.failed(window.transfer, error);
            }
            function onLastShareProgressChanged() {
                window.transfer = Share.event(window.transfer, HandoverService.lastShareProgress, false);
            }
            function onLastShareResultChanged() {
                window.transfer = Share.event(window.transfer, HandoverService.lastShareResult, true);
            }
            function onConnectedChanged() {
                if (!HandoverService.connected)
                    window.transfer = Share.disconnected(window.transfer);
            }
            function onSharesInterrupted() {
                window.transfer = Share.disconnected(window.transfer);
            }
        }
    }
}
