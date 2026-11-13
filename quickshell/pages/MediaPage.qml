import ".."
import "../components"
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell

Rectangle {
    id: mediaCard

    required property var window

    Layout.fillWidth: true
    Layout.fillHeight: window.page === "media"
    radius: 12
    color: "#1b2532"
    visible: window.page === "media"

    ScrollView {
        anchors.fill: parent
        anchors.margins: 20
        contentWidth: availableWidth
        clip: true
        ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

        ColumnLayout {
            width: parent.width
            spacing: 16

            Text {
                text: "Laptop audio to phone"
                color: "#ffffff"
                font.pixelSize: 17
            }

            Repeater {
                model: HandoverService.audioStreams

                delegate: RowLayout {
                    required property var modelData
                    property var device: HandoverService.devices.find((d) => {
                        return d.id === modelData.device_id;
                    })
                    property bool active: modelData.state === "starting" || modelData.state === "streaming"

                    Layout.fillWidth: true

                    Text {
                        Layout.fillWidth: true
                        color: "#b9c2cf"
                        text: (device ? device.name : "Phone") + " · " + modelData.state
                        wrapMode: Text.WordWrap
                        textFormat: Text.PlainText
                    }

                    ActionButton {
                        text: active ? "Pause stream" : "Start stream"
                        enabled: !!device && device.connected && device.paired && !HandoverService.pendingCommand
                        onClicked: HandoverService.sendCommand(active ? "audio.stop" : "audio.start", {
                            "device_id": modelData.device_id
                        })
                    }

                }

            }

            Text {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: "#b9c2cf"
                text: HandoverService.audioStreams.length === 0 ? "Connect a phone with laptop audio support." : "Uses the phone's media output. Bluetooth headphones add delay."
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 1
                color: "#354154"
            }

            Text {
                text: "Phone playback"
                color: "#edf1f7"
                font.pixelSize: 17
                font.weight: Font.DemiBold
            }

            Repeater {
                model: HandoverService.mediaSessions

                delegate: ColumnLayout {
                    required property var modelData

                    Layout.fillWidth: true
                    spacing: 4

                    Text {
                        Layout.fillWidth: true
                        color: "#b9c2cf"
                        text: modelData.application + " · " + modelData.playback
                        textFormat: Text.PlainText
                        wrapMode: Text.WordWrap
                    }

                    Text {
                        Layout.fillWidth: true
                        color: "#ffffff"
                        font.pixelSize: 15
                        elide: Text.ElideRight
                        text: modelData.title || "Untitled"
                    }

                    RowLayout {
                        Layout.fillWidth: true

                        ActionButton {
                            text: "Previous"
                            enabled: (modelData.controls || []).includes("previous")
                            onClicked: HandoverService.mediaCommand(modelData, "previous")
                        }

                        ActionButton {
                            text: modelData.playback === "playing" ? "Pause" : "Play"
                            enabled: (modelData.controls || []).includes("play_pause") || (modelData.playback === "playing" ? (modelData.controls || []).includes("pause") : (modelData.controls || []).includes("play"))
                            onClicked: {
                                let action = "play_pause";
                                if (!(modelData.controls || []).includes("play_pause"))
                                    action = modelData.playback === "playing" ? "pause" : "play";

                                HandoverService.mediaCommand(modelData, action);
                            }
                        }

                        ActionButton {
                            text: "Next"
                            enabled: (modelData.controls || []).includes("next")
                            onClicked: HandoverService.mediaCommand(modelData, "next")
                        }

                    }

                }

            }

            EmptyState {
                Layout.fillWidth: true
                visible: HandoverService.mediaSessions.length === 0
                title: "Nothing playing on your phone"
                description: "Start playback on the phone to see its available media controls here."
            }

        }

    }

}
