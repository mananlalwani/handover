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

                delegate: Rectangle {
                    required property var modelData
                    property var device: HandoverService.devices.find((d) => {
                        return d.id === modelData.device_id;
                    })
                    property bool hasIntent: modelData.enabled !== undefined && modelData.enabled !== null
                    property bool active: hasIntent ? !!modelData.enabled : (modelData.state === "starting" || modelData.state === "streaming")
                    property bool retrying: modelData.state === "failed" && active
                    property var diagnostics: modelData.diagnostics || ({})
                    property double currentTimeMs: Date.now()

                    Layout.fillWidth: true
                    implicitHeight: details.implicitHeight + 20
                    radius: 8
                    color: "#202b3b"

                    Timer {
                        interval: 1000
                        repeat: true
                        running: true
                        onTriggered: parent.currentTimeMs = Date.now()
                    }

                    ColumnLayout {
                        id: details
                        anchors.fill: parent
                        anchors.margins: 10
                        spacing: 8

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 8

                            Text {
                                Layout.fillWidth: true
                                color: "#edf1f7"
                                text: (device ? device.name : "Phone") + " · " + modelData.state + (hasIntent ? (active ? " · enabled" : " · stopped") : " · intent unavailable")
                                wrapMode: Text.WordWrap
                                textFormat: Text.PlainText
                            }

                            ActionButton {
                                text: retrying ? "Retry stream" : active ? "Stop stream" : "Start stream"
                                enabled: retrying ? !!device && device.connected && device.paired && !HandoverService.pendingCommand : active ? !HandoverService.pendingCommand : !!device && device.connected && device.paired && !HandoverService.pendingCommand
                                onClicked: HandoverService.sendCommand(active && !retrying ? "audio.stop" : "audio.start", {
                                    "device_id": modelData.device_id
                                })
                            }

                            ActionButton {
                                visible: retrying
                                text: "Stop recovery"
                                enabled: !HandoverService.pendingCommand
                                onClicked: HandoverService.sendCommand("audio.stop", { device_id: modelData.device_id })
                            }
                            ChoiceBox {
                                id: profileChoice
                                Layout.preferredWidth: 138
                                model: ["balanced", "music", "low_delay"]
                                enabled: modelData.profile !== undefined && modelData.profile !== null && !!device && device.connected && !HandoverService.pendingCommand

                                Binding {
                                    target: profileChoice
                                    property: "currentIndex"
                                    value: profileChoice.model.indexOf(modelData.profile)
                                }

                                contentItem: Text {
                                    leftPadding: 8
                                    rightPadding: 24
                                    text: profileChoice.currentText === "low_delay" ? "Low delay" : profileChoice.currentText === "" ? "Profile unavailable" : profileChoice.currentText.charAt(0).toUpperCase() + profileChoice.currentText.slice(1)
                                    color: profileChoice.enabled ? "#edf1f7" : "#8793a3"
                                    verticalAlignment: Text.AlignVCenter
                                    elide: Text.ElideRight
                                }
                                background: Rectangle {
                                    radius: 6
                                    color: profileChoice.enabled ? "#354154" : "#293444"
                                    border.color: "#536176"
                                }
                                onActivated: {
                                    const requestedProfile = currentText;
                                    currentIndex = profileChoice.model.indexOf(modelData.profile);
                                    HandoverService.sendCommand("audio.profile", {
                                        "device_id": modelData.device_id,
                                        "profile": requestedProfile
                                    });
                                }
                            }
                        }

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 4

                            Text {
                                Layout.fillWidth: true
                                visible: modelData.retry_attempt > 0
                                color: "#e5b96b"
                                wrapMode: Text.WordWrap
                                text: "Reconnect attempt " + modelData.retry_attempt
                            }

                            Text {
                                Layout.fillWidth: true
                                visible: !!modelData.failure
                                color: "#f08a8a"
                                wrapMode: Text.WordWrap
                                text: "Stream failure: " + (typeof modelData.failure === "string" ? modelData.failure.replace(/_/g, " ") : JSON.stringify(modelData.failure))
                            }

                            Text {
                                Layout.fillWidth: true
                                visible: !!diagnostics.sender
                                color: "#b9c2cf"
                                wrapMode: Text.WordWrap
                                text: diagnostics.sender ? "Sender target · " + Math.round(diagnostics.sender.encoder_bitrate_bps / 1000) + " kb/s · " + diagnostics.sender.packet_duration_ms + " ms packets · dropped " + diagnostics.sender.capture_dropped_packets + " · updated " + age(diagnostics.sender.updated_at_ms) : ""
                            }

                            Text {
                                Layout.fillWidth: true
                                visible: !!diagnostics.receiver
                                color: "#b9c2cf"
                                wrapMode: Text.WordWrap
                                text: receiverSummary(diagnostics.receiver)
                            }

                            Text {
                                Layout.fillWidth: true
                                visible: !!diagnostics.receiver
                                color: "#8e9bad"
                                wrapMode: Text.WordWrap
                                text: "RTT measures the network round trip. Buffering is a lifetime receiver average. Neither measures total audible or Bluetooth delay."
                            }
                        }
                    }

                    function age(timestamp) {
                        if (timestamp === undefined || timestamp === null) return "age unavailable";
                        const seconds = Math.max(0, Math.floor((currentTimeMs - timestamp) / 1000));
                        return seconds < 60 ? seconds + "s ago" : Math.floor(seconds / 60) + "m ago";
                    }

                    function receiverSummary(receiver) {
                        const parts = ["Receiver · route " + receiver.route];
                        if (receiver.transport) parts.push(receiver.transport.toUpperCase());
                        parts.push(receiver.round_trip_ms === null || receiver.round_trip_ms === undefined ? "RTT unmeasured" : "RTT " + receiver.round_trip_ms + " ms");
                        if (receiver.jitter_ms !== null && receiver.jitter_ms !== undefined) parts.push("jitter " + receiver.jitter_ms + " ms");
                        if (receiver.jitter_buffer_ms !== null && receiver.jitter_buffer_ms !== undefined) parts.push("avg buffer " + receiver.jitter_buffer_ms + " ms");
                        if (receiver.bitrate_bps !== null && receiver.bitrate_bps !== undefined) parts.push(Math.round(receiver.bitrate_bps / 1000) + " kb/s");
                        if (receiver.packets_received !== null && receiver.packets_received !== undefined) parts.push("received " + receiver.packets_received);
                        if (receiver.packets_lost !== null && receiver.packets_lost !== undefined) parts.push("lost " + receiver.packets_lost);
                        parts.push(age(receiver.updated_at_ms));
                        return parts.join(" · ");
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
