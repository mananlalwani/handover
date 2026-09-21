import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell

Rectangle {
    required property var window
    id: contactsCard
    Layout.fillWidth: true
    Layout.fillHeight: true
    radius: 8
    color: "#303741"
    visible: window.page === "contacts"

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 12
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
            Button {
                text: "Refresh"
                onClicked: HandoverService.refreshContacts()
            }
            Button {
                text: "Sync phone"
                enabled: window.appDevices.some(device => device.connected && device.paired)
                onClicked: {
                    const device = window.appDevices.find(item => item.connected && item.paired);
                    if (device)
                        HandoverService.syncContacts(device);
                }
            }
        }

        ListView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: HandoverService.contacts
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
                    source: "data:image/jpeg;base64," + (modelData.photo || "")
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
