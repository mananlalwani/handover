import ".."
import "../components"
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell

Rectangle {
    id: filesCard

    required property var window
    readonly property var listing: HandoverService.filesystemResult && HandoverService.filesystemResult.device_id === window.selectedDeviceId ? HandoverService.filesystemResult : null

    Layout.fillWidth: true
    Layout.fillHeight: true
    radius: 12
    color: "#1b2532"
    visible: window.page === "files"

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 20
        spacing: 8

        Text {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            color: "#b9c2cf"
            text: "Browse the files exposed by the selected phone. Select a folder to open it."
        }

        RowLayout {
            Layout.fillWidth: true

            InputField {
                id: phonePath

                Layout.fillWidth: true
                text: "."
                placeholderText: "Path under home"
            }

            ActionButton {
                text: "Open folder"
                enabled: window.selectedDeviceAvailable
                onClicked: {
                    const device = window.selectedDeviceAvailable ? window.selectedDevice : null;
                    if (device)
                        HandoverService.listPhoneDirectory(device, phonePath.text.trim() || ".");

                }
            }

        }

        Text {
            Layout.fillWidth: true
            color: "#ffb4ab"
            visible: !!(filesCard.listing && filesCard.listing.failure)
            text: filesCard.listing && filesCard.listing.failure ? filesCard.listing.failure : ""
        }

        EmptyState {
            Layout.fillWidth: true
            visible: !filesCard.listing || !(filesCard.listing.entries || []).length
            title: filesCard.listing ? "No files to show" : "Browse your phone"
            description: "Choose a connected phone above and open a folder to see its contents."
        }

        ListView {
            spacing: 14
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: filesCard.listing ? (filesCard.listing.entries || []) : []

            ScrollBar.vertical: ScrollBar {
            }

            delegate: ActionButton {
                required property var modelData

                width: ListView.view.width
                text: (modelData.directory ? "Folder · " : "File · ") + modelData.name + (modelData.size !== undefined && modelData.size !== null ? " · " + modelData.size : "")
                enabled: window.selectedDeviceAvailable && modelData.directory
                onClicked: {
                    if (!modelData.directory)
                        return ;

                    const base = phonePath.text.trim() || ".";
                    phonePath.text = base === "." ? modelData.name : base + "/" + modelData.name;
                    const device = window.selectedDeviceAvailable ? window.selectedDevice : null;
                    if (device)
                        HandoverService.listPhoneDirectory(device, phonePath.text);

                }
            }

        }

    }

    Connections {
        function onSelectedDeviceIdChanged() {
            phonePath.text = ".";
        }

        target: window
    }

}
