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
    visible: window.page === "commands"
    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 12
        spacing: 8
        RowLayout {
            Layout.fillWidth: true
            Text {
                Layout.fillWidth: true
                text: "Allowlisted Linux commands"
                color: "#ffffff"
                font.pixelSize: 16
                font.bold: true
            }
            Button {
                text: "Refresh"
                onClicked: HandoverService.listCustomCommands()
            }
        }
        Text {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            color: "#b9c2cf"
            text: "Only names from custom-commands.toml. Running one is accepted, not a guarantee of the program's effect."
        }
        ListView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: HandoverService.customCommands
            delegate: RowLayout {
                required property var modelData
                width: ListView.view.width
                Text {
                    Layout.fillWidth: true
                    color: "#e4e7eb"
                    text: modelData.name + " · " + (modelData.argv || []).join(" ")
                }
                Button {
                    text: "Run"
                    onClicked: HandoverService.runCustomCommand(modelData.name)
                }
            }
        }
        Text {
            Layout.fillWidth: true
            visible: !!HandoverService.customResult
            color: HandoverService.customResult && HandoverService.customResult.accepted
                ? "#a7e3b5" : "#ffb4ab"
            text: HandoverService.customResult
                ? (HandoverService.customResult.accepted
                    ? HandoverService.customResult.name + " ran"
                        + (HandoverService.customResult.exit_code !== undefined
                            ? " · exit " + HandoverService.customResult.exit_code : "")
                    : HandoverService.customResult.name + " rejected")
                : ""
        }
    }
}
