import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import ".."

Rectangle {
    required property var window
    Layout.fillWidth: true
    Layout.fillHeight: true
    radius: 12
    color: "#252d3b"
    visible: window.page === "files"
    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 12
        spacing: 8
        Text { text: "Phone files"; color: "#ffffff"; font.pixelSize: 16; font.bold: true }
        Text {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            color: "#b9c2cf"
            text: "Home-confined listings only. A listed name is not a download."
        }
        RowLayout {
            Layout.fillWidth: true
            TextField {
                id: phonePath
                Layout.fillWidth: true
                text: "."
                placeholderText: "Path under home"
            }
            Button {
                text: "List"
                enabled: window.appDevices.some(device => device.connected && device.paired)
                onClicked: {
                    const device = window.appDevices.find(item => item.connected && item.paired);
                    if (device)
                        HandoverService.listPhoneDirectory(device, phonePath.text.trim() || ".");
                }
            }
        }
        Text {
            Layout.fillWidth: true
            color: "#ffb4ab"
            visible: !!(HandoverService.filesystemResult && HandoverService.filesystemResult.failure)
            text: HandoverService.filesystemResult && HandoverService.filesystemResult.failure
                ? HandoverService.filesystemResult.failure : ""
        }
        ListView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: HandoverService.filesystemResult
                ? (HandoverService.filesystemResult.entries || []) : []
            delegate: Button {
                required property var modelData
                width: ListView.view.width
                text: (modelData.directory ? "📁 " : "📄 ") + modelData.name
                    + (modelData.size !== undefined && modelData.size !== null
                        ? " · " + modelData.size : "")
                onClicked: {
                    if (!modelData.directory)
                        return;
                    const base = phonePath.text.trim() || ".";
                    phonePath.text = base === "." ? modelData.name : base + "/" + modelData.name;
                    const device = window.appDevices.find(item => item.connected && item.paired);
                    if (device)
                        HandoverService.listPhoneDirectory(device, phonePath.text);
                }
            }
        }
    }
}
