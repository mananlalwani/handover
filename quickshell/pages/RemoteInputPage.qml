import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell

Rectangle {
    required property var window
    Layout.fillWidth: true
    implicitHeight: remoteInputColumn.implicitHeight + 24
    radius: 8
    color: "#303741"
    visible: window.page === "remote-input"

    ColumnLayout {
        id: remoteInputColumn
        anchors.fill: parent
        anchors.margins: 12
        spacing: 8

        Text { text: "Remote input"; color: "#ffffff"; font.pixelSize: 16; font.bold: true }
        Text { text: "Use the buttons to control the selected native phone."; color: "#c6cbd6" }
        RowLayout {
            Layout.fillWidth: true
            Button { text: "Left click"; onClicked: HandoverService.remoteInput(window.appDevices[0], "click", 0, 0, 1, "") }
            Button { text: "Right click"; onClicked: HandoverService.remoteInput(window.appDevices[0], "click", 0, 0, 3, "") }
            Button { text: "Scroll up"; onClicked: HandoverService.remoteInput(window.appDevices[0], "scroll", 0, -1, 0, "") }
            Button { text: "Scroll down"; onClicked: HandoverService.remoteInput(window.appDevices[0], "scroll", 0, 1, 0, "") }
        }
        RowLayout {
            Layout.fillWidth: true
            TextField { id: remoteText; Layout.fillWidth: true; placeholderText: "Text to type on Linux" }
            Button {
                text: "Type"
                enabled: remoteText.text.length > 0
                onClicked: {
                    HandoverService.remoteInput(window.appDevices[0], "type", 0, 0, 0, remoteText.text);
                    remoteText.clear();
                }
            }
        }
    }
}
