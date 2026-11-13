import ".."
import "../components"
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell

Rectangle {
    required property var window

    Layout.fillWidth: true
    Layout.fillHeight: true
    radius: 8
    color: "#1b2532"
    visible: window.page === "clipboard"

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 20
        spacing: 8

        Text {
            Layout.fillWidth: true
            text: "Send text"
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
            Layout.minimumHeight: 70
            placeholderText: "Text to send to the phone"
            wrapMode: TextArea.Wrap
        }

        ActionButton {
            Layout.fillWidth: true
            text: "Send to phone"
            enabled: window.selectedDeviceAvailable && clipboardText.text.length > 0 && clipboardText.text.length <= 32768
            onClicked: {
                const device = window.selectedDeviceAvailable ? window.selectedDevice : null;
                if (device && HandoverService.sendClipboard(device, clipboardText.text))
                    clipboardText.clear();

            }
        }

        ActionButton {
            Layout.fillWidth: true
            text: "Send current desktop clipboard"
            enabled: window.selectedDeviceAvailable
            onClicked: {
                const device = window.selectedDeviceAvailable ? window.selectedDevice : null;
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

            ActionButton {
                text: "Refresh"
                onClicked: HandoverService.refreshClipboardHistory()
            }

            ActionButton {
                text: "Clear"
                onClicked: HandoverService.clearClipboardHistory(false)
            }

        }

        EmptyState {
            Layout.fillWidth: true
            visible: HandoverService.clipboardHistory.length === 0
            title: "Clipboard history is empty"
            description: "Copied text will appear here when clipboard history is available."
        }

        ListView {
            spacing: 14
            Layout.fillWidth: true
            Layout.preferredHeight: 160
            Layout.maximumHeight: parent.height * 0.35
            clip: true
            model: HandoverService.clipboardHistory

            ScrollBar.vertical: ScrollBar {
            }

            delegate: RowLayout {
                required property var modelData

                width: ListView.view.width

                Text {
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                    color: "#e4e7eb"
                    text: (modelData.pinned ? "★ " : "") + modelData.text
                }

                ActionButton {
                    text: "Copy"
                    onClicked: HandoverService.copyClipboardHistory(modelData.id)
                }

                ActionButton {
                    text: modelData.pinned ? "Unpin" : "Pin"
                    onClicked: HandoverService.pinClipboardHistory(modelData.id, !modelData.pinned)
                }

            }

        }

    }

}
