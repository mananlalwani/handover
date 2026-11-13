import ".."
import "../Share.js" as Share
import "../components"
import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts

Rectangle {
    id: transfersPage

    required property var window
    property string resource: ""
    readonly property bool canShare: !!window.selectedDevice && Share.devices([window.selectedDevice]).length > 0
    property var transfer: null
    property string transferDeviceName: ""
    readonly property bool locked: !!transfer && transfer.phase !== "done"

    function submit() {
        if (locked || !HandoverService.connected || HandoverService.pendingCommand)
            return ;

        const device = canShare ? window.selectedDevice : null;
        const item = Share.resource(resource);
        if (!device || !item)
            return ;

        transferDeviceName = device.name;
        transfer = Share.start(device.id, item.method);
        const sent = item.method === "share.file" ? HandoverService.sendFile(device, item.value) : HandoverService.sendUrl(device, item.value);
        if (!sent)
            transfer = Share.failed(transfer, HandoverService.lastError);

    }

    function cancelTransfer() {
        if (!Share.canCancel(transfer) || !HandoverService.connected || HandoverService.pendingCommand)
            return ;

        const current = transfer;
        transfer = Share.requestCancellation(current);
        if (!HandoverService.cancelShare({
            "id": current.deviceId
        }, current.transferId))
            transfer = Share.cancellationResult(transfer, current.deviceId, current.transferId, false, HandoverService.lastError);

    }

    color: "#1b2532"
    radius: 12
    visible: window.page === "transfers"

    FileDialog {
        id: fileDialog

        title: "Choose a file to send"
        fileMode: FileDialog.OpenFile
        onAccepted: transfersPage.resource = selectedFile.toString()
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 20
        spacing: 12

        Label {
            text: "File or link"
            color: "#ffffff"
            font.pixelSize: 17
            font.bold: true
        }

        Label {
            Layout.fillWidth: true
            text: transfersPage.transfer ? "Transfer to " + transfersPage.transferDeviceName : !HandoverService.connected ? "Waiting for the Handover daemon…" : !transfersPage.canShare ? "Select a connected phone that supports sharing." : "Send to " + window.selectedDevice.name
            color: "#b9c2cf"
            wrapMode: Text.WordWrap
        }

        RowLayout {
            InputField {
                Layout.fillWidth: true
                text: transfersPage.resource
                placeholderText: "Paste an http or https link"
                enabled: !transfersPage.locked
                onTextEdited: transfersPage.resource = text
                selectByMouse: true
            }

            ActionButton {
                text: "Choose file"
                enabled: !transfersPage.locked
                onClicked: fileDialog.open()
            }

        }

        Label {
            Layout.fillWidth: true
            text: transfersPage.transfer ? transfersPage.transfer.text : "Choose one file or paste a link."
            textFormat: Text.PlainText
            wrapMode: Text.WrapAnywhere
            color: "#b9c2cf"
        }

        Label {
            Layout.fillWidth: true
            visible: !!HandoverService.lastReceivedShare
            text: HandoverService.lastReceivedShare ? "Last received: " + (HandoverService.lastReceivedShare.resource.kind === "url" ? HandoverService.lastReceivedShare.resource.url : HandoverService.lastReceivedShare.resource.path) : ""
            textFormat: Text.PlainText
            wrapMode: Text.WrapAnywhere
            color: "#aab7c8"
        }

        Item {
            Layout.fillHeight: true
        }

        RowLayout {
            ActionButton {
                text: "Send"
                primary: true
                enabled: HandoverService.connected && transfersPage.canShare && !!Share.resource(transfersPage.resource) && !transfersPage.locked && !HandoverService.pendingCommand
                onClicked: transfersPage.submit()
            }

            ActionButton {
                text: "Cancel queued transfer"
                visible: Share.canCancel(transfersPage.transfer) || (!!transfersPage.transfer && transfersPage.transfer.cancelPending)
                enabled: Share.canCancel(transfersPage.transfer) && HandoverService.connected && !HandoverService.pendingCommand
                onClicked: transfersPage.cancelTransfer()
            }

            ActionButton {
                text: "New share"
                visible: !!transfersPage.transfer && transfersPage.transfer.phase !== "pending" && !transfersPage.transfer.cancelPending
                onClicked: transfersPage.transfer = null
            }

            Item {
                Layout.fillWidth: true
            }

        }

        Label {
            Layout.fillWidth: true
            text: "Leaving this page does not cancel an accepted transfer."
            wrapMode: Text.WordWrap
            color: "#b9c2cf"
            font.pixelSize: 12
        }

    }

    Connections {
        function onShareAccepted(method, deviceId, transferId) {
            transfersPage.transfer = Share.accepted(transfersPage.transfer, method, deviceId, transferId);
        }

        function onShareFinished(method, deviceId, success, error, unknown) {
            if (!success && transfersPage.transfer && transfersPage.transfer.deviceId === deviceId && transfersPage.transfer.method === method)
                transfersPage.transfer = Share.failed(transfersPage.transfer, error, unknown);

        }

        function onShareCancelFinished(deviceId, transferId, cancelled, error, unknown) {
            transfersPage.transfer = Share.cancellationResult(transfersPage.transfer, deviceId, transferId, cancelled, error, unknown);
        }

        function onLastShareProgressChanged() {
            transfersPage.transfer = Share.event(transfersPage.transfer, HandoverService.lastShareProgress, false);
        }

        function onLastShareResultChanged() {
            transfersPage.transfer = Share.event(transfersPage.transfer, HandoverService.lastShareResult, true);
        }

        function onConnectedChanged() {
            if (!HandoverService.connected)
                transfersPage.transfer = Share.disconnected(transfersPage.transfer);

        }

        function onSharesInterrupted() {
            transfersPage.transfer = Share.disconnected(transfersPage.transfer);
        }

        target: HandoverService
    }

}
