import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import ".."

Rectangle {
    required property var window
    id: mediaCard
    Layout.fillWidth: true
    Layout.fillHeight: window.page === "media"
    implicitHeight: window.page === "overview" ? 180 : 280
    radius: 12
    color: "#252d3b"
    visible: window.page === "overview" || window.page === "media"

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 12
        spacing: 8
        Text {
            text: "Media"
            color: "#ffffff"
            font.pixelSize: 16
            font.bold: true
        }
        ListView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: HandoverService.mediaSessions
            delegate: ColumnLayout {
                required property var modelData
                width: ListView.view.width
                spacing: 4
                Text {
                    Layout.fillWidth: true
                    color: "#b9c2cf"
                    text: modelData.application + " · " + modelData.playback
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
                    Button {
                        text: "Previous"
                        enabled: (modelData.controls || []).includes("previous")
                        onClicked: HandoverService.mediaCommand(modelData, "previous")
                    }
                    Button {
                        text: modelData.playback === "playing" ? "Pause" : "Play"
                        enabled: (modelData.controls || []).includes("play_pause")
                            || (modelData.playback === "playing"
                                ? (modelData.controls || []).includes("pause")
                                : (modelData.controls || []).includes("play"))
                        onClicked: {
                            let action = "play_pause";
                            if (!(modelData.controls || []).includes("play_pause"))
                                action = modelData.playback === "playing" ? "pause" : "play";
                            HandoverService.mediaCommand(modelData, action);
                        }
                    }
                    Button {
                        text: "Next"
                        enabled: (modelData.controls || []).includes("next")
                        onClicked: HandoverService.mediaCommand(modelData, "next")
                    }
                }
            }
        }
        Text {
            visible: HandoverService.mediaSessions.length === 0
            color: "#b9c2cf"
            text: "No active media sessions"
        }
    }
}
