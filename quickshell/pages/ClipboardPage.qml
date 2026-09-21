import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell

Rectangle {
    required property var window
    Layout.fillWidth: true
    Layout.fillHeight: true
    radius: 8
    color: "#303741"
    visible: window.page === "clipboard"

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 12
        spacing: 8
        Text {
            Layout.fillWidth: true
            text: "Explicit clipboard transfer"
            color: "#ffffff"
            font.pixelSize: 16
            font.bold: true
        }
        Text {
            Layout.fillWidth: true
            text: "Enter text to put on the selected phone. This does not mirror the desktop clipboard."
            color: "#b9c2cf"
            wrapMode: Text.WordWrap
        }
        TextArea {
            id: clipboardText
            Layout.fillWidth: true
            Layout.fillHeight: true
            placeholderText: "Text to send to the phone"
            wrapMode: TextArea.Wrap
        }
        Button {
            Layout.fillWidth: true
            text: "Send to phone"
            enabled: window.appDevices.some(device => device.connected && device.paired)
                && clipboardText.text.length > 0
                && clipboardText.text.length <= 32768
            onClicked: {
                const device = window.appDevices.find(item => item.connected && item.paired);
                if (device && HandoverService.sendClipboard(device, clipboardText.text))
                    clipboardText.clear();
            }
        }
        Button {
            Layout.fillWidth: true
            text: "Send current desktop clipboard"
            enabled: window.appDevices.some(device => device.connected && device.paired)
            onClicked: {
                const device = window.appDevices.find(item => item.connected && item.paired);
                if (device)
                    HandoverService.sendCurrentClipboard(device);
            }
        }
        Switch {
            text: "Mirror desktop clipboard to the phone"
            checked: HandoverService.clipboardMirrorEnabled
            onToggled: HandoverService.setClipboardMirror(checked)
        }
        RowLayout {
            Layout.fillWidth: true
            Text {
                Layout.fillWidth: true
                text: "History"
                color: "#ffffff"
                font.bold: true
            }
            Button {
                text: "Refresh"
                onClicked: HandoverService.refreshClipboardHistory()
            }
            Button {
                text: "Clear"
                onClicked: HandoverService.clearClipboardHistory(false)
            }
        }
        ListView {
            Layout.fillWidth: true
            Layout.preferredHeight: 180
            clip: true
            model: HandoverService.clipboardHistory
            delegate: RowLayout {
                required property var modelData
                width: ListView.view.width
                Text {
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                    color: "#e4e7eb"
                    text: (modelData.pinned ? "★ " : "") + modelData.text
                }
                Button {
                    text: "Copy"
                    onClicked: HandoverService.copyClipboardHistory(modelData.id)
                }
                Button {
                    text: modelData.pinned ? "Unpin" : "Pin"
                    onClicked: HandoverService.pinClipboardHistory(modelData.id, !modelData.pinned)
                }
            }
        }
    }
}
