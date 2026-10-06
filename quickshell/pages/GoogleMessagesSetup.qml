import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import Quickshell.Io
import ".."

Dialog {
    id: setup
    title: "Connect Google Messages"
    modal: true
    closePolicy: Popup.NoAutoClose
    width: 560
    property var browsers: []
    property string browserPath: ""
    property string notice: "Choose an installed browser."
    property string savedAccount: ""
    property bool receivedStatus: false
    property bool cancelling: false
    property string verificationSymbol: ""

    function prepare() {
        savedAccount = "";
        verificationSymbol = "";
        browserPath = "";
        browsers = [];
        cancelling = false;
        notice = "Looking for installed browsers...";
        open();
        inventory.running = true;
    }
    function updateConnection() {
        if (savedAccount && HandoverService.messagingAccounts.some(account =>
            account.id === savedAccount && account.connected && account.authenticated))
            notice = "Google Messages is connected. Setup is complete.";
    }
    Connections {
        target: HandoverService
        function onMessagingAccountsChanged() { setup.updateConnection(); }
    }
    FileDialog {
        id: browserDialog
        title: "Choose a Chromium browser executable"
        fileMode: FileDialog.OpenFile
        onAccepted: {
            const url = selectedFile.toString();
            if (url.startsWith("file://")) {
                setup.browserPath = decodeURIComponent(url.slice(7));
                setup.notice = "Selected browser ready.";
            }
        }
    }
    Process {
        id: inventory
        command: ["handover-google-messages-setup", "--list-browsers"]
        stdout: SplitParser {
            onRead: data => {
                try {
                    const result = JSON.parse(data);
                    if (Array.isArray(result.browsers)) {
                        setup.browsers = result.browsers;
                        setup.browserPath = result.browsers.length ? result.browsers[0].path : "";
                        setup.notice = result.browsers.length ? "Choose a browser, then continue."
                            : "Install a Chromium browser, or choose its executable.";
                    }
                } catch (_) {
                    setup.notice = "Could not read the browser list.";
                }
            }
        }
        onExited: (exitCode, exitStatus) => {
            if (exitCode !== 0)
                setup.notice = "Install Handover's optional Google Messages setup component to continue.";
        }
    }
    Process {
        id: connectProcess
        command: ["handover-google-messages-setup", "--browser", setup.browserPath]
        stdout: SplitParser {
            onRead: data => {
                try {
                    const event = JSON.parse(data);
                    if (typeof event.verification === "string")
                        setup.verificationSymbol = event.verification;
                    if (event.status === "saved" || event.status === "failed")
                        setup.verificationSymbol = "";
                    if (typeof event.message === "string") {
                        setup.receivedStatus = true;
                        setup.notice = event.message;
                    }
                    if (event.status === "saved" && typeof event.account === "string")
                        setup.savedAccount = event.account;
                    setup.updateConnection();
                } catch (_) {
                    setup.notice = "Could not read setup progress.";
                }
            }
        }
        onExited: (exitCode, exitStatus) => {
            setup.verificationSymbol = "";
            if (!setup.receivedStatus)
                setup.notice = "Google Messages setup could not start.";
            else if (exitCode !== 0 && setup.savedAccount)
                setup.notice = "Credentials were saved. Waiting for Handover to reconnect.";
            setup.updateConnection();
        }
    }
    contentItem: ColumnLayout {
        spacing: 12
        Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            text: "Sign in normally, then choose Exit in the browser menu. Handover finishes setup and asks for phone confirmation when needed."
        }
        RowLayout {
            Layout.fillWidth: true
            ComboBox {
                Layout.fillWidth: true
                model: setup.browsers
                textRole: "name"
                enabled: !connectProcess.running && !inventory.running
                onActivated: index => setup.browserPath = setup.browsers[index].path
            }
            Button {
                text: "Choose executable"
                enabled: !connectProcess.running
                onClicked: browserDialog.open()
            }
        }
        Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            text: "Your normal browser profile stays separate. Google authentication is saved in your desktop credential store. Messages pause during setup; other Handover features keep working."
        }
        Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
            text: setup.notice
        }
        Label {
            Layout.fillWidth: true
            visible: setup.verificationSymbol.length > 0
            text: "Confirm " + setup.verificationSymbol + " on your phone."
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
            font.pixelSize: 24
        }
        RowLayout {
            Layout.alignment: Qt.AlignRight
            Button {
                text: connectProcess.running ? "Cancel setup" : setup.savedAccount ? "Done" : "Close"
                enabled: !connectProcess.running || !setup.cancelling
                onClicked: {
                    if (connectProcess.running) {
                        setup.cancelling = true;
                        connectProcess.signal(2);
                    } else
                        setup.close();
                }
            }
            Button {
                text: "Continue"
                visible: !setup.savedAccount
                enabled: !!setup.browserPath && !connectProcess.running && !inventory.running
                    && !setup.savedAccount
                onClicked: {
                    setup.receivedStatus = false;
                    setup.verificationSymbol = "";
                    setup.notice = "Starting Google Messages setup...";
                    connectProcess.running = true;
                }
            }
        }
    }
}
