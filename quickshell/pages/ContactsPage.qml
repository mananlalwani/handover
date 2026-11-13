import ".."
import "../components"
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell

Rectangle {
    id: contactsCard

    required property var window

    Layout.fillWidth: true
    Layout.fillHeight: true
    radius: 8
    color: "#1b2532"
    visible: window.page === "contacts"

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 20
        spacing: 8

        RowLayout {
            Layout.fillWidth: true

            Text {
                Layout.fillWidth: true
                text: "Contacts"
                color: "#ffffff"
                font.pixelSize: 16
                font.bold: true
            }

            ActionButton {
                text: "Refresh"
                onClicked: HandoverService.refreshContacts()
            }

            ActionButton {
                text: "Sync phone"
                enabled: window.selectedDeviceAvailable
                onClicked: {
                    const device = window.selectedDeviceAvailable ? window.selectedDevice : null;
                    if (device)
                        HandoverService.syncContacts(device);

                }
            }

        }

        Text {
            Layout.fillWidth: true
            visible: HandoverService.contactsSyncError.length > 0
            text: HandoverService.contactsSyncError
            color: "#ffb4ab"
            wrapMode: Text.Wrap
        }

        EmptyState {
            Layout.fillWidth: true
            visible: HandoverService.contacts.length === 0
            title: "No contacts synced"
            description: "Choose a connected phone above, then use Sync phone. Allow Contacts access in the phone app first."
        }

        ListView {
            spacing: 14
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: HandoverService.contacts

            ScrollBar.vertical: ScrollBar {
            }

            delegate: ColumnLayout {
                required property var modelData

                width: ListView.view.width
                spacing: 2

                Text {
                    Layout.fillWidth: true
                    text: modelData.display_name
                    color: "#f3f4f6"
                    font.bold: true
                }

                Image {
                    Layout.preferredWidth: 42
                    Layout.preferredHeight: 42
                    source: modelData.photo ? "data:image/jpeg;base64," + modelData.photo : ""
                    visible: !!modelData.photo
                    fillMode: Image.PreserveAspectCrop
                }

                Text {
                    Layout.fillWidth: true
                    text: (modelData.phones || []).concat(modelData.emails || []).join(" · ")
                    color: "#b9c2cf"
                    elide: Text.ElideRight
                }

            }

        }

    }

}
