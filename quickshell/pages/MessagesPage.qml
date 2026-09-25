import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import Quickshell
import ".."

Rectangle {
    id: messagesCard
    required property var window
    anchors.fill: parent
    anchors.margins: 0
    radius: 8
    color: "#303741"
    visible: window.page === "messages"
    property var selectedConversation: null
    property var replyingTo: null
    property string status: ""
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
    property var selectedAccount: modernAccountPicker.count > 0
        ? HandoverService.messagingAccounts[modernAccountPicker.currentIndex] : null
    property var accountConversations: selectedAccount
        ? HandoverService.conversations.filter(item =>
            item.id.account_id === selectedAccount.id).slice().sort((a, b) =>
                (b.last_activity_at || 0) - (a.last_activity_at || 0))
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
    onSelectedConversationChanged: {
        modernMessageList.followTail = true;
        Qt.callLater(() => modernMessageList.positionViewAtEnd());
    }

    function conversationLabel(conversation) {
        if (!conversation)
            return "Select a conversation";
        if (conversation.title)
            return conversation.title;
        const others = conversation.participants.filter(item => !item.is_self);
        return others.map(item =>
            HandoverService.contactLabel(item)).join(", ");
    }

    function senderLabel(sender) {
        return HandoverService.contactLabel(sender);
    }

    function messageTimestamp(message) {
        if (!message || !message.sent_at)
            return "";
        return Qt.formatDateTime(
            new Date(Number(message.sent_at) / 1000),
            "MMM d, yyyy · h:mm AP");
    }

    ColumnLayout {
        id: legacyMessagesLayout
        anchors.fill: parent
        anchors.margins: 12
        spacing: 6
        visible: false

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
                property var messageData: modelData
                implicitHeight: bubble.implicitHeight + 8

                Rectangle {
                    anchors.left: messageData.sender.is_self ? undefined : parent.left
                    anchors.right: messageData.sender.is_self ? parent.right : undefined
                    width: Math.min(parent.width * 0.82, 520)
                    height: bubble.implicitHeight
                    radius: 14
                    color: messageData.sender.is_self ? "#245b8f" : "#252d39"
                }

                ColumnLayout {
                    id: bubble
                    anchors.left: messageData.sender.is_self ? undefined : parent.left
                    anchors.right: messageData.sender.is_self ? parent.right : undefined
                    width: Math.min(parent.width * 0.82, 520)
                    spacing: 3
                    anchors.margins: 10

                    Text {
                        Layout.fillWidth: true
                        color: "#ffffff"
                        font.pixelSize: 14
                        textFormat: Text.PlainText
                        wrapMode: Text.Wrap
                        text: (messageData.deleted ? "[deleted] " : "")
                            + messagesCard.senderLabel(messageData.sender) + ": "
                            + (messageData.text || "")
                            + ((messageData.attachments || []).length > 0
                                ? " [" + (messageData.attachments || []).map(item =>
                                    item.name || item.local_id).join(", ") + "]" : "")
                    }

                    Text {
                        Layout.fillWidth: true
                        color: "#b7c6d9"
                        font.pixelSize: 12
                        textFormat: Text.PlainText
                        visible: messageData.reply_to !== undefined && messageData.reply_to !== null
                            || (messageData.reactions || []).length > 0
                        text: {
                            const reply = messageData.reply_to
                                ? "reply to " + messageData.reply_to.local_id + " " : "";
                            const reactions = (messageData.reactions || []).map(item =>
                                item.emoji + "×" + item.count).join(" ");
                            return reply + reactions;
                        }
                    }

                    Row {
                        spacing: 4
                        visible: {
                            const conversation = messagesCard.accountConversations.find(item =>
                                HandoverService.sameConversationId(
                                    item.id, messageData.id.conversation_id));
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
                                const reacted = (messageData.reactions || []).some(item =>
                                    item.emoji === modelData
                                    && item.participant_ids.includes("self"));
                                if (reacted)
                                    HandoverService.unreact(
                                        messageData.id.conversation_id,
                                        messageData.id.local_id, modelData);
                                else
                                    HandoverService.react(
                                        messageData.id.conversation_id,
                                        messageData.id.local_id, modelData);
                            }
                        }
                    }
                    Button {
                        text: "Reply"
                        flat: true
                        enabled: !HandoverService.pendingMessaging
                        onClicked: messagesCard.replyingTo = messageData.id.local_id
                    }
                    Button {
                        text: "Delete"
                        flat: true
                        visible: messageData.sender.is_self
                        enabled: !HandoverService.pendingMessaging
                        onClicked: {
                            HandoverService.deleteMessage(
                                messageData.id.conversation_id, messageData.id.local_id);
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
Item {
    id: modernMessages
    parent: messagesCard
    anchors.fill: parent
    anchors.margins: 14
    visible: window.page === "messages"

    ColumnLayout {
        anchors.fill: parent
        spacing: 10

        RowLayout {
            Layout.fillWidth: true
            spacing: 8
            Text {
                text: "Messages"
                color: "#f3f4f6"
                font.pixelSize: 24
                font.bold: true
            }
            Item { Layout.fillWidth: true }
            ComboBox {
                id: modernAccountPicker
                Layout.preferredWidth: 190
                model: HandoverService.messagingAccounts.map(item => item.label)
                onCurrentIndexChanged: {
                    messagesCard.selectedConversation = null;
                    messagesCard.replyingTo = null;
                }
            }
            Button {
                text: "↻"
                enabled: modernAccountPicker.count > 0
                    && !HandoverService.pendingMessaging
                onClicked: HandoverService.syncAccount(
                    HandoverService.messagingAccounts[modernAccountPicker.currentIndex].id)
            }
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 10

            Rectangle {
                Layout.preferredWidth: 240
                Layout.fillHeight: true
                radius: 12
                color: "#202733"

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 8
                    spacing: 6
                    TextField {
                        id: conversationSearch
                        Layout.fillWidth: true
                        placeholderText: "Search conversations"
                        background: Rectangle {
                            radius: 8
                            color: "#151a23"
                            border.color: "#344154"
                        }
                    }
                    ListView {
                        id: modernConversationList
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        model: messagesCard.accountConversations.filter(item =>
                            !conversationSearch.text.trim()
                            || messagesCard.conversationLabel(item).toLowerCase().includes(
                                conversationSearch.text.trim().toLowerCase()))
                        delegate: Item {
                            required property var modelData
                            width: modernConversationList.width
                            height: 58
                            Rectangle {
                                anchors.fill: parent
                                radius: 8
                                color: messagesCard.selectedConversation !== null
                                    && HandoverService.sameConversationId(
                                        messagesCard.selectedConversation, modelData.id)
                                    ? "#314b6c" : "transparent"
                            }
                            Rectangle {
                                x: 8
                                y: 9
                                width: 40
                                height: 40
                                radius: 20
                                color: "#526d9b"
                                Text {
                                    anchors.centerIn: parent
                                    text: messagesCard.conversationLabel(modelData).charAt(0).toUpperCase()
                                    color: "#ffffff"
                                    font.pixelSize: 18
                                     font.bold: true
                                 }
                             }
                             Image {
                                 anchors.left: parent.left
                                 anchors.leftMargin: 8
                                 anchors.verticalCenter: parent.verticalCenter
                                 width: 40
                                 height: 40
                                 source: {
                                     const participant = modelData.participants
                                         .find(item => !item.is_self);
                                     return HandoverService.contactPhoto(participant);
                                 }
                                 fillMode: Image.PreserveAspectCrop
                                 visible: source !== ""
                                 z: 2
                             }
                             Column {
                                anchors.left: parent.left
                                anchors.leftMargin: 58
                                anchors.right: parent.right
                                anchors.rightMargin: 10
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 2
                                Text {
                                    width: parent.width
                                    text: messagesCard.conversationLabel(modelData)
                                    color: "#f3f4f6"
                                    elide: Text.ElideRight
                                    font.pixelSize: 14
                                }
                                Text {
                                    text: modelData.transport
                                    color: "#9caec5"
                                    font.pixelSize: 11
                                }
                            }
                            Rectangle {
                                visible: !!modelData.unread_count
                                anchors.right: parent.right
                                anchors.rightMargin: 8
                                anchors.verticalCenter: parent.verticalCenter
                                width: 20
                                height: 20
                                radius: 10
                                color: "#4e9bea"
                                Text {
                                    anchors.centerIn: parent
                                    text: modelData.unread_count || ""
                                    color: "#ffffff"
                                    font.pixelSize: 11
                                }
                            }
                            MouseArea {
                                anchors.fill: parent
                                onClicked: {
                                    messagesCard.selectedConversation = modelData.id;
                                    messagesCard.replyingTo = null;
                                    HandoverService.loadHistory(modelData.id, 20);
                                    HandoverService.markRead(modelData.id);
                                }
                            }
                        }
                    }
                    TextField {
                        id: modernOpenField
                        Layout.fillWidth: true
                        placeholderText: "Phone or email"
                        visible: modernAccountPicker.count > 0
                    }
                    Button {
                        Layout.fillWidth: true
                        text: "New conversation"
                        enabled: modernOpenField.text.trim().length > 0
                            && !HandoverService.pendingMessaging
                        onClicked: {
                            if (HandoverService.openConversation(
                                messagesCard.selectedAccount.id,
                                [modernOpenField.text.trim()]))
                                modernOpenField.text = "";
                        }
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.fillHeight: true
                radius: 12
                color: "#1d2430"

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 12
                    spacing: 8
                    RowLayout {
                        Layout.fillWidth: true
                        Text {
                            Layout.fillWidth: true
                            text: messagesCard.selectedConversation
                                ? messagesCard.conversationLabel(
                                    messagesCard.accountConversations.find(item =>
                                        HandoverService.sameConversationId(
                                            item.id, messagesCard.selectedConversation)))
                                : "Select a conversation"
                            color: "#f3f4f6"
                            font.pixelSize: 18
                            font.bold: true
                        }
                        Text {
                            text: messagesCard.selectedTyping ? "typing…" : ""
                            color: "#8db8e8"
                        }
                    }
                    ListView {
                        id: modernMessageList
                        property bool followTail: true
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        spacing: 8
                        model: messagesCard.selectedMessages
                        onCountChanged: {
                            if (followTail)
                                Qt.callLater(() => positionViewAtEnd());
                        }
                        onContentHeightChanged: {
                            if (followTail)
                                Qt.callLater(() => positionViewAtEnd());
                        }
                        onContentYChanged: {
                            if (moving && !atYEnd)
                                followTail = false;
                            else if (atYEnd)
                                followTail = true;
                        }
                        onMovementEnded: followTail = atYEnd
                        ScrollBar.vertical: ScrollBar {
                            policy: ScrollBar.AsNeeded
                            implicitWidth: 14
                            minimumSize: 0.1
                            contentItem: Rectangle {
                                implicitWidth: 10
                                radius: 5
                                color: parent.pressed ? "#b8c8dc"
                                    : parent.hovered ? "#91a6c0" : "#647991"
                            }
                        }
                        delegate: Item {
                            required property var modelData
                            property var messageData: modelData
                            width: modernMessageList.width
                            height: modernBubble.implicitHeight + 4
                            Rectangle {
                                id: bubbleBackground
                                anchors.left: modelData.sender.is_self ? undefined : parent.left
                                anchors.right: modelData.sender.is_self ? parent.right : undefined
                                width: Math.min(parent.width * 0.78, 500)
                                height: modernBubble.implicitHeight
                                radius: 14
                                color: modelData.transport === "sms"
                                    ? (modelData.sender.is_self ? "#765126" : "#493a2b")
                                    : (modelData.sender.is_self ? "#28649b" : "#2a3442")
                            }
                            ColumnLayout {
                                id: modernBubble
                                anchors.left: modelData.sender.is_self ? undefined : parent.left
                                anchors.right: modelData.sender.is_self ? parent.right : undefined
                                width: Math.min(parent.width * 0.78, 500)
                                spacing: 3
                                Text {
                                    Layout.fillWidth: true
                                    Layout.leftMargin: 12
                                    Layout.rightMargin: 12
                                    Layout.topMargin: 10
                                    text: {
                                        const conversation = messagesCard.accountConversations.find(item =>
                                            HandoverService.sameConversationId(
                                                item.id, modelData.id.conversation_id));
                                        const group = conversation && conversation.kind === "group";
                                        return (!modelData.sender.is_self || group)
                                            ? messagesCard.senderLabel(modelData.sender) : "You";
                                    }
                                    color: "#b9d7f2"
                                    wrapMode: Text.Wrap
                                    font.pixelSize: 11
                                    font.bold: true
                                }
                                Text {
                                    Layout.fillWidth: true
                                    Layout.leftMargin: 12
                                    Layout.rightMargin: 12
                                    text: modelData.text
                                        || ((modelData.attachments || []).length > 0
                                            ? "📎 " + (modelData.attachments || []).map(item =>
                                                item.name || item.mime || "attachment").join(", ")
                                            : "")
                                    color: "#ffffff"
                                    wrapMode: Text.Wrap
                                    font.pixelSize: 14
                                }
                                Repeater {
                                    model: modelData.attachments || []
                                    Button {
                                        required property var modelData
                                        Layout.leftMargin: 8
                                        Layout.rightMargin: 8
                                        text: "📎 " + (modelData.name || modelData.mime || "Open attachment")
                                        flat: true
                                        enabled: !!modelData.staged_path
                                        onClicked: Qt.openUrlExternally(
                                            "file://" + modelData.staged_path)
                                    }
                                }
                                Text {
                                    Layout.fillWidth: true
                                    Layout.leftMargin: 12
                                    Layout.rightMargin: 12
                                    visible: (modelData.reactions || []).length > 0
                                    text: (modelData.reactions || []).map(item =>
                                        item.emoji + " ×" + item.count).join("  ")
                                    color: "#c4d8ec"
                                    font.pixelSize: 12
                                }
                                Row {
                                    Layout.leftMargin: 8
                                    Layout.rightMargin: 8
                                    Layout.bottomMargin: 6
                                    spacing: 4
                                    Text {
                                        anchors.verticalCenter: parent.verticalCenter
                                        text: messagesCard.messageTimestamp(modelData)
                                        color: "#aebed0"
                                        font.pixelSize: 10
                                    }
                                    Button {
                                        text: "Reply"
                                        flat: true
                                        onClicked: messagesCard.replyingTo = modelData.id.local_id
                                    }
                                    Repeater {
                                        model: ["❤", "👍", "😂"]
                                        Button {
                                            required property var modelData
                                            text: modelData
                                            flat: true
                                            onClicked: HandoverService.react(
                                                messagesCard.selectedConversation,
                                                messageData.id.local_id,
                                                modelData)
                                        }
                                    }
                                }
                            }
                        }
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        visible: messagesCard.selectedConversation !== null
                        spacing: 6
                        Text {
                            Layout.fillWidth: true
                            color: messagesCard.replyingTo !== null
                                ? "#9fc5e8" : "#9caec5"
                            elide: Text.ElideRight
                            text: {
                                if (messagesCard.replyingTo !== null)
                                    return "Replying to " + messagesCard.replyingTo;
                                if (HandoverService.isLoadingAllHistory(
                                        messagesCard.selectedConversation))
                                    return "Loading all messages…";
                                if (messagesCard.status.length > 0)
                                    return messagesCard.status;
                                if (messagesCard.selectedMessages.length === 0)
                                    return "Loading messages…";
                                return messagesCard.selectedMessages.length + " messages";
                            }
                        }
                        ToolButton {
                            text: "⋯"
                            onClicked: messageOptions.open()
                            Menu {
                                id: messageOptions
                                MenuItem {
                                    text: "Load older messages"
                                    enabled: !HandoverService.pendingMessaging
                                        && HandoverService.conversationCursors[
                                            messagesCard.selectedKey] !== undefined
                                    onTriggered: HandoverService.loadHistory(
                                        messagesCard.selectedConversation, 20,
                                        HandoverService.conversationCursors[
                                            messagesCard.selectedKey])
                                }
                                MenuItem {
                                    text: "Load all messages"
                                    enabled: !HandoverService.isLoadingAllHistory(
                                        messagesCard.selectedConversation)
                                        && !HandoverService.pendingMessaging
                                    onTriggered: {
                                        messagesCard.status = "requesting complete history…";
                                        if (!HandoverService.loadAllHistory(
                                            messagesCard.selectedConversation))
                                            messagesCard.status = HandoverService.lastError;
                                    }
                                }
                                MenuItem {
                                    text: "Cancel reply"
                                    visible: messagesCard.replyingTo !== null
                                    onTriggered: messagesCard.replyingTo = null
                                }
                            }
                        }
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        TextField {
                            id: modernComposeField
                            Layout.fillWidth: true
                            placeholderText: "Write a message…"
                            background: Rectangle {
                                radius: 18
                                color: "#151a23"
                                border.color: "#3a4a60"
                            }
                            enabled: messagesCard.selectedConversation !== null
                                && !HandoverService.pendingMessaging
                            onAccepted: modernSendButton.clicked()
                        }
                        Button {
                            id: modernSendButton
                            text: "↑"
                            font.pixelSize: 18
                            font.bold: true
                            enabled: modernComposeField.text.trim().length > 0
                                && messagesCard.selectedConversation !== null
                                && !HandoverService.pendingMessaging
                            onClicked: {
                                if (HandoverService.sendText(
                                    messagesCard.selectedConversation,
                                    modernComposeField.text,
                                    messagesCard.replyingTo)) {
                                    modernComposeField.text = "";
                                    messagesCard.replyingTo = null;
                                }
                            }
                            background: Rectangle {
                                radius: 18
                                color: modernSendButton.enabled ? "#3d8bd9" : "#303947"
                            }
                        }
                        Button {
                            text: "＋  Attach"
                            enabled: messagesCard.selectedConversation !== null
                                && !HandoverService.pendingMessaging
                            onClicked: attachmentDialog.open()
                        }
                    }
                }
            }
        }
    }
}

}
