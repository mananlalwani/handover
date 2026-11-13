import ".."
import "../components"
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Rectangle {
    required property var window

    radius: 12
    color: "#1b2532"
    visible: window.page === "remote-input"

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 20
        spacing: 16

        Text {
            text: "Phone input"
            color: "#edf1f7"
            font.pixelSize: 17
            font.bold: true
        }

        Text {
            Layout.fillWidth: true
            text: "Send clicks, scrolling, or text to the selected phone. The phone must support remote input."
            color: "#aab7c8"
            wrapMode: Text.WordWrap
        }

        EmptyState {
            Layout.fillWidth: true
            visible: !window.selectedDeviceAvailable
            title: "No connected phone selected"
            description: "Choose a connected phone from the device picker above."
        }

        Flow {
            Layout.fillWidth: true
            spacing: 8
            enabled: window.selectedDeviceAvailable

            ActionButton {
                text: "Left click"
                onClicked: HandoverService.remoteInput(window.selectedDevice, "click", 0, 0, 1, "")
            }

            ActionButton {
                text: "Right click"
                onClicked: HandoverService.remoteInput(window.selectedDevice, "click", 0, 0, 3, "")
            }

            ActionButton {
                text: "Scroll up"
                onClicked: HandoverService.remoteInput(window.selectedDevice, "scroll", 0, -1, 0, "")
            }

            ActionButton {
                text: "Scroll down"
                onClicked: HandoverService.remoteInput(window.selectedDevice, "scroll", 0, 1, 0, "")
            }

        }

        RowLayout {
            Layout.fillWidth: true

            InputField {
                id: remoteText

                Layout.fillWidth: true
                placeholderText: "Text to type on the phone"
                maximumLength: 512
                enabled: window.selectedDeviceAvailable
            }

            ActionButton {
                text: "Type text"
                enabled: window.selectedDeviceAvailable && remoteText.text.length > 0
                onClicked: {
                    if (HandoverService.remoteInput(window.selectedDevice, "type", 0, 0, 0, remoteText.text))
                        remoteText.clear();

                }
            }

        }

        Item {
            Layout.fillHeight: true
        }

    }

}
