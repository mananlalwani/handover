import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import Quickshell
import ".."

Rectangle {
    id: callCard
    required property var window
    anchors.fill: parent
    radius: 8
    color: "#303741"
    property var phone: window.appDevices.find(device => device.connected) || null
    property var call: phone ? HandoverService.calls.find(state => state.device_id === phone.id) || null : null
    property string dialDevice: ""
    property string dialAddress: ""
    onVisibleChanged: if (visible) HandoverService.refreshCallAudio()
    onCallChanged: if (visible) HandoverService.refreshCallAudio()
    Dialog {
        id: confirmCall
        width: 360
        title: "Place a real phone call?"
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        Label { text: callCard.dialAddress }
        onAccepted: {
            if (callCard.phone && callCard.phone.id === callCard.dialDevice && callCard.supports("place"))
                HandoverService.callControl(callCard.phone, "place", callCard.dialAddress);
        }
    }
    function supports(action) {
        return phone !== null && call !== null && (call.controls || []).includes(action);
    }
    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 18
        spacing: 12
        Text { text: "Calls"; color: "#f3f4f6"; font.pixelSize: 24; font.bold: true }
        Text {
            Layout.fillWidth: true
            text: !callCard.phone ? "Native phone unavailable"
                : !callCard.call ? "Waiting for phone call state — updated Android app required"
                : callCard.call.phase === "off_hook" ? "Dialing or in a call · " + callCard.phone.name
                : callCard.call.phase === "ringing" ? "Incoming call · " + callCard.phone.name
                : callCard.call.phase === "idle" ? "Phone idle · " + callCard.phone.name
                : "Call state unavailable — check phone permission"
            color: "#b9c2cf"
        }
        Text {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            color: "#b9c2cf"
            text: !HandoverService.callAudio || !HandoverService.callAudio.observed
                ? "Local call audio: status unavailable"
                : !HandoverService.callAudio.gateway_ready
                ? "Local call audio: no HFP audio-gateway profile selected"
                : HandoverService.callAudio.duplex_running
                ? "Local HFP input/output running — phone association and microphone/speaker routing not confirmed"
                : "Local HFP profile selected — audio input/output not running"
        }
        Button { text: "Refresh audio status"; onClicked: HandoverService.refreshCallAudio() }
        Text {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            color: "#b9c2cf"
            text: HandoverService.callNotice
            visible: text.length > 0
        }
        TextField {
            id: callAddress
            Layout.fillWidth: true
            placeholderText: "Phone number"
            inputMethodHints: Qt.ImhDialableCharactersOnly
        }
        RowLayout {
            Layout.fillWidth: true
            Button {
                text: "Call"
                enabled: /^\+?[0-9]{1,15}$/.test(callAddress.text.trim()) && callCard.supports("place")
                onClicked: {
                    callCard.dialDevice = callCard.phone.id;
                    callCard.dialAddress = callAddress.text.trim();
                    confirmCall.open();
                }
            }
            Button {
                text: "Answer"
                enabled: callCard.supports("answer")
                onClicked: HandoverService.callControl(callCard.phone, "answer", "")
            }
            Button {
                text: "Decline"
                enabled: callCard.supports("decline")
                onClicked: HandoverService.callControl(callCard.phone, "decline", "")
            }
            Button {
                text: "Hang up"
                enabled: callCard.supports("hangup")
                onClicked: HandoverService.callControl(callCard.phone, "hangup", "")
            }
        }
        Text {
            Layout.fillWidth: true
            text: HandoverService.lastError
            color: "#ffb4ab"
            visible: text.length > 0
        }
        Item { Layout.fillHeight: true }
    }
}
