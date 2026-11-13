import "../components"
import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import Quickshell.Io

Rectangle {
    id: root

    property bool installed: false
    property bool checked: false
    property bool fallbackRequested: false
    property string notice: ""
    readonly property bool showFallback: checked && (!installed || fallbackRequested)
    readonly property bool busy: helper.running
    property string action: "status"
    property bool receivedResult: false
    readonly property string helperPath: decodeURIComponent(Qt.resolvedUrl("../MessagesApp.py").toString().slice(7))

    function run(operation, source) {
        if (helper.running)
            return ;

        action = operation;
        receivedResult = false;
        notice = "";
        helper.command = ["python3", helperPath, operation];
        if (source)
            helper.command = helper.command.concat([source]);

        helper.running = true;
    }

    implicitHeight: content.implicitHeight + 32
    radius: 12
    color: showFallback ? "#302b23" : "#1b2532"
    Component.onCompleted: run("status")

    Process {
        id: helper

        onExited: (code, status) => {
            if (!root.receivedResult) {
                root.checked = true;
                root.fallbackRequested = true;
                root.notice = "Could not check or start Handover Messages. Make sure Python 3 is installed, or use the QML fallback.";
            }
        }
        onRunningChanged: {
            if (!running) {
                Qt.callLater(() => {
                    if (!helper.running && !root.receivedResult) {
                        root.checked = true;
                        root.fallbackRequested = true;
                        root.notice = "The app helper could not run. You can keep using the QML fallback.";
                    }
                });
            }
        }

        stdout: SplitParser {
            onRead: (data) => {
                try {
                    const result = JSON.parse(data);
                    if (typeof result.installed !== "boolean" || typeof result.success !== "boolean")
                        return ;

                    if (root.checked && !root.installed)
                        root.fallbackRequested = true;

                    root.installed = result.installed;
                    root.checked = true;
                    root.receivedResult = true;
                    if (!result.success) {
                        root.fallbackRequested = true;
                        root.notice = result.error || "The app request failed.";
                    } else if (root.action === "install") {
                        root.fallbackRequested = true;
                        root.notice = "Installed for your user. Open Handover Messages when you are ready.";
                        installDialog.close();
                    } else if (root.action === "launch") {
                        root.notice = "Launch requested. Your accounts and history are shared with this window.";
                    }
                } catch (_) {
                    root.notice = "The app helper returned an unreadable result.";
                }
            }
        }

    }

    ColumnLayout {
        id: content

        anchors.centerIn: parent
        width: parent.width - 32
        spacing: 10

        Text {
            Layout.fillWidth: true
            text: root.showFallback ? "QML messaging fallback" : "Use Handover Messages"
            color: root.showFallback ? "#f0d09f" : "#edf1f7"
            font.pixelSize: root.showFallback ? 16 : 23
            font.weight: Font.DemiBold
            wrapMode: Text.WordWrap
        }

        Text {
            Layout.fillWidth: true
            text: !root.checked ? "Checking for the standalone app…" : root.showFallback ? "This is the QML fallback. Features and behavior can differ from the standalone app. Handover Messages is recommended for everyday messaging." : "Open the standalone Messages app for everyday messaging. It uses the same accounts and history as Handover."
            color: "#b8c3d1"
            wrapMode: Text.WordWrap
            font.pixelSize: 13
        }

        Flow {
            Layout.fillWidth: true
            spacing: 8

            ActionButton {
                text: root.installed ? "Open Handover Messages" : "Install Handover Messages…"
                primary: true
                enabled: root.checked && !root.busy
                onClicked: root.installed ? root.run("launch") : installDialog.open()
            }

            ActionButton {
                text: "Use QML fallback"
                visible: root.installed && !root.fallbackRequested
                onClicked: root.fallbackRequested = true
            }

            ActionButton {
                text: "Check again"
                enabled: !root.busy
                onClicked: root.run("status")
            }

        }

        Text {
            Layout.fillWidth: true
            visible: root.notice.length > 0
            text: root.notice
            textFormat: Text.PlainText
            color: "#f0d09f"
            wrapMode: Text.WordWrap
            font.pixelSize: 12
        }

    }

    FileDialog {
        id: appFile

        title: "Choose bin/handover-messages from an extracted release"
        fileMode: FileDialog.OpenFile
        onAccepted: {
            const url = selectedFile.toString();
            if (url.startsWith("file://"))
                installDialog.sourcePath = decodeURIComponent(url.slice(7));

        }
    }

    Dialog {
        id: installDialog

        property string sourcePath: ""

        anchors.centerIn: parent
        width: Math.min(520, root.width - 24)
        modal: true
        title: "Install Handover Messages"
        font.pixelSize: 13
        padding: 16
        palette.window: "#1b2532"
        palette.windowText: "#edf1f7"
        palette.text: "#edf1f7"
        palette.base: "#131b26"
        palette.button: "#263344"
        palette.buttonText: "#edf1f7"
        standardButtons: Dialog.Close

        background: Rectangle {
            color: "#1b2532"
            radius: 12
            border.color: "#354154"
        }

        contentItem: ColumnLayout {
            spacing: 12

            Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "Download the matching Linux release and extract it. Choose bin/handover-messages from that folder, or the executable from your own build. Only the Messages app and its launcher will be installed for your user."
            }

            Flow {
                Layout.fillWidth: true
                spacing: 8

                ActionButton {
                    text: "Download release"
                    onClicked: Qt.openUrlExternally("https://github.com/mananlalwani/handover/releases")
                }

                ActionButton {
                    text: "Choose executable…"
                    onClicked: appFile.open()
                }

            }

            Label {
                Layout.fillWidth: true
                text: installDialog.sourcePath || "No executable selected"
                textFormat: Text.PlainText
                elide: Text.ElideMiddle
            }

            Label {
                Layout.fillWidth: true
                text: "Requires GTK 4.10 and libadwaita 1.4 or newer."
                wrapMode: Text.WordWrap
                color: "#aab7c8"
                font.pixelSize: 12
            }

            ActionButton {
                text: root.busy ? "Installing…" : "Install app"
                primary: true
                enabled: !!installDialog.sourcePath && !root.busy
                onClicked: root.run("install", installDialog.sourcePath)
            }

            Label {
                Layout.fillWidth: true
                visible: root.notice.length > 0
                text: root.notice
                textFormat: Text.PlainText
                wrapMode: Text.WordWrap
            }

        }

    }

}
