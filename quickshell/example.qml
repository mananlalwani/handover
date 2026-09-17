import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import Quickshell

ShellRoot {
    FloatingWindow {
        id: window
        visible: true
        implicitWidth: 760
        implicitHeight: 760
        color: "#151a23"
        title: "Handover"
        property string page: "overview"
        // The reference UI is focused on the native Handover path for now.
        // Keep KDE Connect available in the daemon, but do not mix its
        // compatibility devices (or the development emulator) into this UI.
        property var appDevices: HandoverService.devices.filter(device =>
            device.id.startsWith("native:") && !device.name.includes("sdk_gphone"))
        property var shareDevices: window.appDevices.filter(device =>
            device.connected && device.paired && device.capabilities.includes("file_transfer"))
        property string shareStatus: ""

        FileDialog {
            id: fileDialog
            title: "Send one file to phone"
            fileMode: FileDialog.OpenFile
            onAccepted: {
                const device = window.shareDevices[devicePicker.currentIndex];
                if (device && !HandoverService.sendFile(device, selectedFile.toString()))
                    window.shareStatus = HandoverService.lastError;
            }
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 16
            spacing: 12

            Text {
                Layout.fillWidth: true
                color: "#f4f4f5"
                font.pixelSize: 18
                text: {
                    const connected = window.appDevices.filter(device => device.connected);
                    if (connected.length === 0)
                        return "No connected device";
                    const device = connected[0];
                    const battery = device.battery
                        ? device.battery.percentage + "%"
                        : "battery unavailable";
                    return device.name + " · " + battery;
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 4

                Repeater {
                    model: [
                        { key: "overview", label: "Overview" },
                        { key: "messages", label: "Messages" },
                        { key: "notifications", label: "Notifications" },
                        { key: "media", label: "Media" },
                        { key: "transfers", label: "Transfers" }
                    ]
                    Button {
                        required property var modelData
                        Layout.fillWidth: true
                        text: modelData.label
                        flat: window.page !== modelData.key
                        highlighted: window.page === modelData.key
                        onClicked: window.page = modelData.key
                    }
                }
            }

            Rectangle {
                id: mediaCard
                Layout.fillWidth: true
                implicitHeight: mediaColumn.implicitHeight + 24
                radius: 8
                color: "#303741"
                visible: window.page === "overview" || window.page === "media"
                property var mediaSession: {
                    const playing = HandoverService.mediaSessions.find(session =>
                        session.playback === "playing");
                    return playing || (HandoverService.mediaSessions.length > 0
                        ? HandoverService.mediaSessions[0] : null);
                }

                ColumnLayout {
                    id: mediaColumn
                    anchors.fill: parent
                    anchors.margins: 12
                    spacing: 5

                    Text {
                        Layout.fillWidth: true
                        color: "#b9c2cf"
                        textFormat: Text.PlainText
                        text: {
                            const session = mediaCard.mediaSession;
                            if (!session)
                                return "Phone media";
                            const device = window.appDevices.find(item =>
                                item.id === session.id.device_id);
                            return session.application + " · "
                                + (device ? device.name : session.id.device_id);
                        }
                    }

                    Text {
                        Layout.fillWidth: true
                        color: "#ffffff"
                        font.pixelSize: 16
                        textFormat: Text.PlainText
                        elide: Text.ElideRight
                        text: mediaCard.mediaSession
                            ? (mediaCard.mediaSession.title || "Untitled")
                            : "No active media session"
                    }

                    Text {
                        Layout.fillWidth: true
                        color: "#e4e7eb"
                        textFormat: Text.PlainText
                        elide: Text.ElideRight
                        text: mediaCard.mediaSession
                            ? (mediaCard.mediaSession.artist || "") : ""
                        visible: text.length > 0
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 8

                        Button {
                            text: "Previous"
                            enabled: mediaCard.mediaSession !== null
                                && mediaCard.mediaSession.controls.includes("previous")
                                && !HandoverService.pendingCommand
                            onClicked: HandoverService.mediaCommand(
                                mediaCard.mediaSession, "previous")
                        }

                        Button {
                            text: mediaCard.mediaSession
                                && mediaCard.mediaSession.playback === "playing"
                                ? "Pause" : "Play"
                            enabled: mediaCard.mediaSession !== null
                                && (mediaCard.mediaSession.controls.includes("play_pause")
                                    || (mediaCard.mediaSession.playback === "playing"
                                        ? mediaCard.mediaSession.controls.includes("pause")
                                        : mediaCard.mediaSession.controls.includes("play")))
                                && !HandoverService.pendingCommand
                            onClicked: {
                                const session = mediaCard.mediaSession;
                                if (!session)
                                    return;
                                let action = "play_pause";
                                if (!session.controls.includes("play_pause"))
                                    action = session.playback === "playing" ? "pause" : "play";
                                HandoverService.mediaCommand(session, action);
                            }
                        }

                        Button {
                            text: "Next"
                            enabled: mediaCard.mediaSession !== null
                                && mediaCard.mediaSession.controls.includes("next")
                                && !HandoverService.pendingCommand
                            onClicked: HandoverService.mediaCommand(
                                mediaCard.mediaSession, "next")
                        }
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        visible: mediaCard.mediaSession !== null
                            && mediaCard.mediaSession.controls.includes("seek")
                        spacing: 8

                        Button {
                            text: "−10 s"
                            enabled: !HandoverService.pendingCommand
                            onClicked: HandoverService.mediaCommand(
                                mediaCard.mediaSession, "seek", -10000)
                        }

                        Button {
                            text: "+10 s"
                            enabled: !HandoverService.pendingCommand
                            onClicked: HandoverService.mediaCommand(
                                mediaCard.mediaSession, "seek", 10000)
                        }
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                visible: window.page === "overview" || window.page === "transfers"

                ComboBox {
                    id: devicePicker
                    Layout.fillWidth: true
                    model: window.shareDevices.map(device => device.name)
                    enabled: window.shareDevices.length > 0
                }

                Button {
                    text: "Send file"
                    enabled: devicePicker.enabled && !HandoverService.pendingCommand
                    onClicked: fileDialog.open()
                }
            }

            RowLayout {
                Layout.fillWidth: true
                visible: window.page === "overview" || window.page === "transfers"

                TextField {
                    id: urlField
                    Layout.fillWidth: true
                    placeholderText: "URL to send"
                }

                Button {
                    text: "Send URL"
                    enabled: devicePicker.enabled && urlField.text.length > 0
                        && !HandoverService.pendingCommand
                    onClicked: {
                        const device = window.shareDevices[devicePicker.currentIndex];
                        if (device && !HandoverService.sendUrl(device, urlField.text))
                            window.shareStatus = HandoverService.lastError;
                    }
                }
            }

            Text {
                Layout.fillWidth: true
                color: "#b9c2cf"
                textFormat: Text.PlainText
                text: window.shareStatus
                visible: text.length > 0 && (window.page === "overview"
                    || window.page === "transfers")
            }

            Rectangle {
                id: notificationCard
                Layout.fillWidth: true
                Layout.fillHeight: true
                radius: 8
                color: "#303741"
                visible: window.page === "overview" || window.page === "notifications"
                property var notification: HandoverService.notifications.find(item => item.reply_supported)
                    || (HandoverService.notifications.length > 0
                        ? HandoverService.notifications[0] : null)

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 12
                    spacing: 6

                    Text {
                        Layout.fillWidth: true
                        color: "#b9c2cf"
                        textFormat: Text.PlainText
                        text: {
                            const notification = notificationCard.notification;
                            if (!notification)
                                return "Phone notifications";
                                const device = window.appDevices.find(item =>
                                item.id === notification.id.device_id);
                            return notification.app_name + " · "
                                + (device ? device.name : notification.id.device_id);
                        }
                    }

                    Text {
                        Layout.fillWidth: true
                        color: "#ffffff"
                        font.pixelSize: 17
                        textFormat: Text.PlainText
                        elide: Text.ElideRight
                        text: notificationCard.notification
                            ? notificationCard.notification.title : "No active notifications"
                    }

                    Text {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        color: "#e4e7eb"
                        textFormat: Text.PlainText
                        wrapMode: Text.Wrap
                        elide: Text.ElideRight
                        text: notificationCard.notification
                            ? notificationCard.notification.body : ""
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        visible: notificationCard.notification !== null
                        spacing: 6

                        TextField {
                            id: replyField
                            Layout.fillWidth: true
                            visible: notificationCard.notification
                                && notificationCard.notification.reply_supported
                            placeholderText: "Reply from desktop"
                            enabled: !HandoverService.pendingCommand
                            onAccepted: sendReply()

                            function sendReply() {
                                if (notificationCard.notification && text.trim().length > 0)
                                    HandoverService.reply(notificationCard.notification, text);
                            }
                        }

                        Button {
                            text: "Send"
                            visible: replyField.visible
                            enabled: !HandoverService.pendingCommand && replyField.text.trim().length > 0
                            onClicked: replyField.sendReply()
                        }

                        Button {
                            text: "Dismiss"
                            visible: notificationCard.notification
                                && notificationCard.notification.clearable
                            enabled: !HandoverService.pendingCommand
                            onClicked: HandoverService.dismiss(notificationCard.notification)
                        }
                    }

                    Row {
                        visible: notificationCard.notification
                            && notificationCard.notification.actions.length > 0
                        spacing: 6
                        Repeater {
                            model: notificationCard.notification
                                ? notificationCard.notification.actions : []
                            Button {
                                required property var modelData
                                text: modelData.label
                                enabled: !HandoverService.pendingCommand
                                onClicked: HandoverService.invokeAction(
                                    notificationCard.notification, modelData)
                            }
                        }
                    }

                    Text {
                        Layout.fillWidth: true
                        color: "#ffb4ab"
                        textFormat: Text.PlainText
                        wrapMode: Text.Wrap
                        visible: HandoverService.lastError.length > 0
                        text: HandoverService.lastError
                    }
                }
            }
        }

        FileDialog {
            id: attachmentDialog
            title: "Attach one file"
            fileMode: FileDialog.OpenFile
            onAccepted: {
                if (messagesCard.selectedConversation
                    && !HandoverService.sendAttachment(
                        messagesCard.selectedConversation, selectedFile.toString(), "")) {
                    messagesCard.status = HandoverService.lastError;
                }
            }
        }

        Rectangle {
            id: messagesCard
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumHeight: 320
            radius: 8
            color: "#303741"
            visible: window.page === "messages"
            property var selectedConversation: null
            property var replyingTo: null
            property string status: ""
            property var selectedAccount: accountPicker.count > 0
                ? HandoverService.messagingAccounts[accountPicker.currentIndex] : null
            property var accountConversations: selectedAccount
                ? HandoverService.conversations.filter(item =>
                    item.id.account_id === selectedAccount.id)
                : []
            property string selectedKey: selectedConversation
                ? HandoverService.conversationKey(selectedConversation) : ""
            property var selectedMessages: selectedKey
                && HandoverService.conversationMessages[selectedKey]
                ? HandoverService.conversationMessages[selectedKey] : []
            property var selectedTyping: selectedConversation
                ? HandoverService.typingStates.find(item =>
                    item.conversation_id.account_id === selectedConversation.account_id
                    && item.conversation_id.local_id === selectedConversation.local_id)
                : null
            property var selectedRead: selectedConversation
                ? HandoverService.readStates.find(item =>
                    item.conversation_id.account_id === selectedConversation.account_id
                    && item.conversation_id.local_id === selectedConversation.local_id)
                : null

            function conversationLabel(conversation) {
                if (conversation.title)
                    return conversation.title;
                const others = conversation.participants.filter(item => !item.is_self);
                return others.map(item =>
                    item.display_name || item.address || item.local_id).join(", ");
            }

            function senderLabel(sender) {
                return sender.display_name || sender.address || sender.local_id;
            }

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 12
                spacing: 6

                Text {
                    Layout.fillWidth: true
                    color: "#b9c2cf"
                    textFormat: Text.PlainText
                    text: "Messages"
                }

                Text {
                    Layout.fillWidth: true
                    color: "#ffb4ab"
                    textFormat: Text.PlainText
                    wrapMode: Text.Wrap
                    visible: HandoverService.pairingPrompt !== null
                    text: HandoverService.pairingPrompt
                        ? "Pairing " + HandoverService.pairingPrompt.accountId
                            + ": " + HandoverService.pairingPrompt.prompt
                        : ""
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: HandoverService.messagingAccounts.length > 0
                    spacing: 6

                    ComboBox {
                        id: accountPicker
                        Layout.fillWidth: true
                        model: HandoverService.messagingAccounts.map(item => item.label)
                        onCurrentIndexChanged: {
                            messagesCard.selectedConversation = null;
                            messagesCard.replyingTo = null;
                        }
                    }

                    Button {
                        text: "Sync"
                        enabled: accountPicker.count > 0 && !HandoverService.pendingMessaging
                        onClicked: {
                            const account = HandoverService.messagingAccounts[accountPicker.currentIndex];
                            if (account)
                                HandoverService.syncAccount(account.id);
                        }
                    }
                }

                Text {
                    Layout.fillWidth: true
                    color: "#e4e7eb"
                    textFormat: Text.PlainText
                    visible: HandoverService.messagingAccounts.length === 0
                    text: "No messaging accounts. Pair with `handoverctl messages login`."
                }

                ListView {
                    id: conversationList
                    Layout.fillWidth: true
                    implicitHeight: 96
                    clip: true
                    model: messagesCard.accountConversations
                    delegate: ItemDelegate {
                        required property var modelData
                        required property int index
                        width: conversationList.width
                        highlighted: messagesCard.selectedConversation !== null
                            && HandoverService.sameConversationId(
                                messagesCard.selectedConversation, modelData.id)
                        text: {
                            const unread = modelData.unread_count ? " (" + modelData.unread_count + ")" : "";
                            const kind = modelData.kind === "group" ? " [group]" : "";
                            return messagesCard.conversationLabel(modelData)
                                + " · " + modelData.transport + kind + unread;
                        }
                        onClicked: {
                            messagesCard.selectedConversation = modelData.id;
                            messagesCard.replyingTo = null;
                            HandoverService.loadHistory(modelData.id, 20);
                            HandoverService.markRead(modelData.id);
                        }
                    }
                }

                ListView {
                    id: messageList
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    spacing: 8
                    model: messagesCard.selectedMessages
                    delegate: Item {
                        required property var modelData
                        width: messageList.width
                        property var message: modelData
                        implicitHeight: bubble.implicitHeight + 8

                        Rectangle {
                            anchors.left: message.sender.is_self ? undefined : parent.left
                            anchors.right: message.sender.is_self ? parent.right : undefined
                            width: Math.min(parent.width * 0.82, 520)
                            height: bubble.implicitHeight
                            radius: 14
                            color: message.sender.is_self ? "#245b8f" : "#252d39"
                        }

                        ColumnLayout {
                            id: bubble
                            anchors.left: message.sender.is_self ? undefined : parent.left
                            anchors.right: message.sender.is_self ? parent.right : undefined
                            width: Math.min(parent.width * 0.82, 520)
                            spacing: 3
                            anchors.margins: 10

                            Text {
                                Layout.fillWidth: true
                                color: "#ffffff"
                                font.pixelSize: 14
                                textFormat: Text.PlainText
                                wrapMode: Text.Wrap
                                text: (message.deleted ? "[deleted] " : "")
                                    + messagesCard.senderLabel(message.sender) + ": "
                                    + (message.text || "")
                                    + (message.attachments.length > 0
                                        ? " [" + message.attachments.map(item =>
                                            item.name || item.local_id).join(", ") + "]" : "")
                            }

                            Text {
                                Layout.fillWidth: true
                                color: "#b7c6d9"
                                font.pixelSize: 12
                                textFormat: Text.PlainText
                                visible: message.reply_to !== undefined && message.reply_to !== null
                                    || message.reactions.length > 0
                                text: {
                                    const reply = message.reply_to
                                        ? "reply to " + message.reply_to.local_id + " " : "";
                                    const reactions = message.reactions.map(item =>
                                        item.emoji + "×" + item.count).join(" ");
                                    return reply + reactions;
                                }
                            }

                            Row {
                                spacing: 4
                                visible: {
                                    const conversation = messagesCard.accountConversations.find(item =>
                                        HandoverService.sameConversationId(
                                            item.id, message.id.conversation_id));
                                    return conversation
                                        && conversation.capabilities.includes("reactions");
                                }
                            Repeater {
                                model: ["❤", "👍", "😂"]
                                Button {
                                    required property var modelData
                                    text: modelData
                                    flat: true
                                    enabled: !HandoverService.pendingMessaging
                                    onClicked: {
                                        const reacted = message.reactions.some(item =>
                                            item.emoji === modelData
                                            && item.participant_ids.includes("self"));
                                        if (reacted)
                                            HandoverService.unreact(
                                                message.id.conversation_id,
                                                message.id.local_id, modelData);
                                        else
                                            HandoverService.react(
                                                message.id.conversation_id,
                                                message.id.local_id, modelData);
                                    }
                                }
                            }
                            Button {
                                text: "Reply"
                                flat: true
                                enabled: !HandoverService.pendingMessaging
                                onClicked: messagesCard.replyingTo = message.id.local_id
                            }
                            Button {
                                text: "Delete"
                                flat: true
                                visible: message.sender.is_self
                                enabled: !HandoverService.pendingMessaging
                                onClicked: {
                                    HandoverService.deleteMessage(
                                        message.id.conversation_id, message.id.local_id);
                                }
                            }
                        }
                            }
                        }
                    }

                Text {
                    Layout.fillWidth: true
                    color: "#8b95a5"
                    textFormat: Text.PlainText
                    visible: messagesCard.selectedTyping !== null
                        && messagesCard.selectedTyping !== undefined
                    text: "typing…"
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: messagesCard.selectedConversation !== null
                        && HandoverService.conversationCursors[messagesCard.selectedKey] !== undefined
                    spacing: 6

                    Button {
                        text: "Load older"
                        enabled: !HandoverService.pendingMessaging
                        onClicked: HandoverService.loadHistory(
                            messagesCard.selectedConversation, 20,
                            HandoverService.conversationCursors[messagesCard.selectedKey])
                    }
                }

                Text {
                    Layout.fillWidth: true
                    color: "#8b95a5"
                    textFormat: Text.PlainText
                    visible: messagesCard.replyingTo !== null
                    text: "Replying to " + (messagesCard.replyingTo || "")
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: messagesCard.selectedConversation !== null
                    spacing: 6

                    TextField {
                        id: composeField
                        Layout.fillWidth: true
                        placeholderText: "Message"
                        enabled: !HandoverService.pendingMessaging
                        onAccepted: sendComposed()
                        function sendComposed() {
                            if (!messagesCard.selectedConversation || text.trim().length === 0)
                                return;
                            if (HandoverService.sendText(
                                messagesCard.selectedConversation, text,
                                messagesCard.replyingTo)) {
                                text = "";
                                messagesCard.replyingTo = null;
                            } else {
                                messagesCard.status = HandoverService.lastError;
                            }
                        }
                    }

                    Button {
                        text: "Send"
                        enabled: composeField.text.trim().length > 0
                            && !HandoverService.pendingMessaging
                        onClicked: composeField.sendComposed()
                    }

                    Button {
                        text: "Attach"
                        enabled: !HandoverService.pendingMessaging
                        onClicked: attachmentDialog.open()
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: messagesCard.selectedAccount !== null
                    spacing: 6

                    TextField {
                        id: openField
                        Layout.fillWidth: true
                        placeholderText: "Start conversation: +15550001"
                        enabled: !HandoverService.pendingMessaging
                    }

                    Button {
                        text: "Start"
                        enabled: openField.text.trim().length > 0
                            && !HandoverService.pendingMessaging
                        onClicked: {
                            if (HandoverService.openConversation(
                                messagesCard.selectedAccount.id, [openField.text.trim()]))
                                openField.text = "";
                            else
                                messagesCard.status = HandoverService.lastError;
                        }
                    }
                }

                Text {
                    Layout.fillWidth: true
                    color: "#b9c2cf"
                    textFormat: Text.PlainText
                    visible: messagesCard.status.length > 0
                        || (messagesCard.selectedRead !== null
                            && messagesCard.selectedRead !== undefined)
                    text: messagesCard.status.length > 0 ? messagesCard.status
                        : (messagesCard.selectedRead && messagesCard.selectedRead.unread
                            ? "unread" : "read");
                }
            }
        }

        Connections {
            target: HandoverService
            function onCommandFinished(method, notificationId, success, error) {
                if (method === "notification.reply" && success)
                    replyField.text = "";
            }
            function onShareFinished(method, deviceId, success, error) {
                window.shareStatus = success
                    ? "Accepted by KDE Connect; delivery is not confirmed"
                    : error;
            }
            function onMessagingFinished(method, requestId, success, error) {
                messagesCard.status = success ? "accepted: " + requestId : error;
            }
            function onMessagingAccepted(method, subject, success, error) {
                messagesCard.status = success ? "accepted" : error;
            }
        }
    }
}
