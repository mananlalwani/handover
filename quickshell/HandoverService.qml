pragma Singleton

import QtQml
import Quickshell
import Quickshell.Io

Singleton {
    id: root

    readonly property int protocolVersion: 1
    readonly property string socketPath: String(Quickshell.env("XDG_RUNTIME_DIR") || "")
        + "/handover/handoverd.sock"
    readonly property bool connected: socketLoader.item
        ? socketLoader.item.connected
        : false
    property var devices: []
    property var notifications: []
    property var mediaSessions: []
    property var calls: []
    property var contacts: []
    property var callAudio: null
    property string callNotice: ""
    function refreshCallAudio() {
        callAudio = null;
        sendRequest("calls.audio", {});
    }
    function refreshContacts() {
        return sendRequest("contacts.list", {});
    }

    function syncContacts(device) {
        if (!device || !device.id || !device.id.startsWith("native:")) {
            lastError = "native phone is unavailable";
            return false;
        }
        return sendRequest("contacts.sync", { device_id: device.id });
    }

    function contactForParticipant(participant) {
        if (!participant)
            return null;
        const address = String(participant.address || "").trim();
        if (!address)
            return null;
        const normalized = address.replace(/[^0-9]/g, "");
        const email = address.toLowerCase();
        return contacts.find(contact =>
            (normalized.length > 0 && (contact.phones || []).some(phone =>
                String(phone).replace(/[^0-9]/g, "") === normalized))
            || (email.includes("@") && (contact.emails || []).some(candidate =>
                String(candidate).toLowerCase() === email))) || null;
    }

    function contactLabel(participant) {
        const contact = contactForParticipant(participant);
        return contact ? contact.display_name
            : (participant.display_name || participant.address || participant.local_id);
    }

    function contactPhoto(participant) {
        const contact = contactForParticipant(participant);
        return contact && contact.photo ? "data:image/jpeg;base64," + contact.photo : "";
    }
    property var lastReceivedShare: null
    property var lastShareResult: null
    property var lastShareProgress: null
    property var filesystemResult: null
    property var clipboardHistory: []
    property bool clipboardMirrorEnabled: false
    property var customCommands: []
    property var customResult: null
    property var pendingPeers: []
    property var snapshotBuffer: null
    property string lastError: ""
    property string nativeNotice: ""
    property bool nativePending: false
    property var pendingCommand: null
    signal commandFinished(string method, var notificationId, bool success, string error)
    signal shareFinished(string method, string deviceId, bool success, string error)
    signal mediaFinished(string action, var mediaId, bool success, string error)
    // Messaging state. Conversations belong to messaging accounts, never to
    // physical devices. Message windows are keyed by "account:thread".
    property var messagingAccounts: []
    property var conversations: []
    property var conversationMessages: ({})
    property var conversationCursors: ({})
    property var exhaustiveHistoryLoads: ({})
    property var typingStates: []
    property var readStates: []
    property var pairingPrompt: null
    property var pendingMessaging: null
    signal messagingFinished(string method, string requestId, bool success, string error)
    signal messagingAccepted(string method, string subject, bool success, string error)
    signal historyProgress(var conversationId, bool complete, int messageCount)

    function conversationKey(id) {
        return id.account_id + ":" + id.local_id;
    }

    function sameConversationId(first, second) {
        return first.account_id === second.account_id && first.local_id === second.local_id;
    }

    function sameMessageId(first, second) {
        return sameConversationId(first.conversation_id, second.conversation_id)
            && first.local_id === second.local_id;
    }

    function replaceConversation(conversation) {
        const next = conversations.filter(existing => !sameConversationId(existing.id, conversation.id));
        next.push(conversation);
        conversations = next;
    }

    function removeConversation(conversationId) {
        conversations = conversations.filter(existing => !sameConversationId(existing.id, conversationId));
        const key = conversationKey(conversationId);
        const messages = Object.assign({}, conversationMessages);
        delete messages[key];
        conversationMessages = messages;
        const cursors = Object.assign({}, conversationCursors);
        delete cursors[key];
        conversationCursors = cursors;
    }

    function replaceMessages(conversationId, incoming, cursorNext) {
        const key = conversationKey(conversationId);
        const known = conversationMessages[key] || [];
        const merged = known.filter(existing =>
            !incoming.some(next => next.id.local_id === existing.id.local_id));
        for (const message of incoming)
            merged.push(message);
        merged.sort((a, b) => (a.sent_at || 0) - (b.sent_at || 0)
            || (a.id.local_id < b.id.local_id ? -1 : 1));
        const messages = Object.assign({}, conversationMessages);
        messages[key] = merged;
        conversationMessages = messages;
        if (cursorNext !== undefined) {
            const cursors = Object.assign({}, conversationCursors);
            if (cursorNext)
                cursors[key] = cursorNext;
            else
                delete cursors[key];
            conversationCursors = cursors;
        }
    }

    function removeMessage(messageId) {
        const key = conversationKey(messageId.conversation_id);
        const known = conversationMessages[key] || [];
        const messages = Object.assign({}, conversationMessages);
        messages[key] = known.filter(existing => !sameMessageId(existing.id, messageId));
        conversationMessages = messages;
    }

    function replaceTyping(state) {
        const next = typingStates.filter(existing =>
            !sameConversationId(existing.conversation_id, state.conversation_id));
        if (state.participant_ids && state.participant_ids.length > 0)
            next.push(state);
        typingStates = next;
    }

    function replaceReadState(state) {
        const next = readStates.filter(existing =>
            !sameConversationId(existing.conversation_id, state.conversation_id));
        next.push(state);
        readStates = next;
    }

    function sendMessaging(method, fields) {
        if (pendingMessaging) {
            lastError = "another messaging command is still pending";
            return false;
        }
        if (!sendRequest(method, fields)) {
            lastError = "handoverd is disconnected";
            return false;
        }
        pendingMessaging = { method: method };
        lastError = "";
        return true;
    }

    function finishMessaging(success, error, requestId, subject) {
        if (!pendingMessaging)
            return;
        const method = pendingMessaging.method;
        pendingMessaging = null;
        if (requestId !== undefined)
            messagingFinished(method, requestId, success, error);
        else
            messagingAccepted(method, subject || "", success, error);
    }

    function refreshMessaging() {
        sendRequest("messages.accounts", null);
    }

    function loadConversations(accountId) {
        sendRequest("messages.conversations", { account_id: accountId });
    }

    function loadHistory(conversationId, limit, cursor) {
        const fields = { conversation_id: conversationId };
        if (limit)
            fields.limit = limit;
        if (cursor)
            fields.cursor = cursor;
        return sendRequest("messages.history", fields);
    }

    function loadAllHistory(conversationId) {
        const key = conversationKey(conversationId);
        const loads = Object.assign({}, exhaustiveHistoryLoads);
        loads[key] = true;
        exhaustiveHistoryLoads = loads;
        // Start from the newest page. A cursor held by the UI may belong to
        // an older daemon window after reconnect/sync and would force an
        // unnecessary slow recovery request. Subsequent pages use only
        // cursors returned by the current request chain.
        const sent = loadHistory(conversationId, 100);
        if (!sent) {
            delete loads[key];
            exhaustiveHistoryLoads = loads;
            lastError = "history request could not be sent";
        }
        return sent;
    }

    function isLoadingAllHistory(conversationId) {
        if (!conversationId)
            return false;
        return !!exhaustiveHistoryLoads[conversationKey(conversationId)];
    }

    function sendText(conversationId, text, replyTo) {
        const fields = { conversation_id: conversationId, text: text };
        if (replyTo)
            fields.reply_to = { conversation_id: conversationId, local_id: replyTo };
        return sendMessaging("messages.send", fields);
    }

    function sendAttachment(conversationId, fileUrl, caption) {
        const fields = { conversation_id: conversationId, file_url: fileUrl };
        if (caption)
            fields.caption = caption;
        return sendMessaging("messages.send_file", fields);
    }

    function react(conversationId, messageLocalId, emoji) {
        return sendMessaging("messages.react", {
            message_id: { conversation_id: conversationId, local_id: messageLocalId },
            emoji: emoji
        });
    }

    function unreact(conversationId, messageLocalId, emoji) {
        return sendMessaging("messages.unreact", {
            message_id: { conversation_id: conversationId, local_id: messageLocalId },
            emoji: emoji
        });
    }

    function markRead(conversationId) {
        return sendMessaging("messages.read", { conversation_id: conversationId });
    }

    function sendTyping(conversationId) {
        return sendMessaging("messages.typing", { conversation_id: conversationId });
    }

    function callControl(device, action, address) {
        if (!device || !device.id || !device.id.startsWith("native:")) {
            lastError = "native phone is unavailable";
            return false;
        }
        const fields = { device_id: device.id, action: action };
        if (address)
            fields.address = address;
        if (!sendRequest("calls.control", fields)) {
            lastError = "handoverd is disconnected";
            return false;
        }
        lastError = "";
        return true;
    }

    function deleteMessage(conversationId, messageLocalId) {
        return sendMessaging("messages.delete", {
            message_id: { conversation_id: conversationId, local_id: messageLocalId }
        });
    }

    function openConversation(accountId, addresses) {
        return sendMessaging("messages.open", {
            account_id: accountId,
            addresses: addresses
        });
    }

    function syncAccount(accountId) {
        return sendMessaging("messages.sync", { account_id: accountId });
    }

    function logoutAccount(accountId) {
        return sendMessaging("messages.logout", { account_id: accountId });
    }

    function sendRequest(method, fields, socketOverride) {
        const socket = socketOverride || socketLoader.item;
        if (!socket || !socket.connected)
            return false;
        socket.write(JSON.stringify(Object.assign({ protocol: protocolVersion, method: method }, fields || {})) + "\n");
        socket.flush();
        return true;
    }

    function sendCommand(method, fields) {
        if (pendingCommand) {
            lastError = "another notification command is still pending";
            return false;
        }
        if (!sendRequest(method, fields)) {
            lastError = "handoverd is disconnected";
            return false;
        }
        pendingCommand = { method: method, notificationId: fields.notification_id };
        lastError = "";
        return true;
    }

    function dismiss(notification) {
        return sendCommand("notification.dismiss", { notification_id: notification.id });
    }

    function invokeAction(notification, action) {
        return sendCommand("notification.action", {
            notification_id: notification.id,
            action_id: action.id
        });
    }

    function reply(notification, text) {
        return sendCommand("notification.reply", {
            notification_id: notification.id,
            text: text
        });
    }

    function sendShare(method, device, fields) {
        if (pendingCommand) {
            lastError = "another command is still pending";
            return false;
        }
        if (!sendRequest(method, Object.assign({ device_id: device.id }, fields))) {
            lastError = "handoverd is disconnected";
            return false;
        }
        pendingCommand = { method: method, deviceId: device.id };
        lastError = "";
        return true;
    }

    function sendUrl(device, url) {
        return sendShare("share.url", device, { url: url });
    }

    function sendFile(device, fileUrl) {
        return sendShare("share.file", device, { file_url: fileUrl });
    }

    function cancelShare(device, transferId) {
        return sendShare("share.cancel", device, { transfer_id: transferId });
    }

    function setScreensaver(action) {
        if (action !== "inhibit" && action !== "release" && action !== "follow") {
            lastError = "screensaver action must be inhibit, release, or follow";
            return false;
        }
        return sendRequest("screensaver." + action, {});
    }

    function setPhoneAwake(device, inhibit) {
        if (!device || !device.id || !device.id.startsWith("native:")) {
            lastError = "native phone is unavailable";
            return false;
        }
        const id = device.id.substring("native:".length);
        return sendRequest("native.keep_awake", { id: id, inhibit: inhibit });
    }

    function openPhoneTethering(device) {
        if (!device || !device.id || !device.id.startsWith("native:")) {
            lastError = "native phone is unavailable";
            return false;
        }
        const id = device.id.substring("native:".length);
        return sendRequest("native.tethering", { id: id });
    }

    function listCustomCommands() {
        return sendRequest("custom.commands", {});
    }

    function runCustomCommand(name) {
        if (!name || name.length === 0 || name.length > 64) {
            lastError = "custom command name must be between 1 and 64 characters";
            return false;
        }
        return sendRequest("custom.run", { name: name });
    }

    function setClipboardMirror(enable) {
        return sendRequest("clipboard.mirror", { enable: enable });
    }

    function clipboardMirrorStatus() {
        return sendRequest("clipboard.mirror_status", {});
    }

    function listPendingPeers() {
        return sendRequest("native.pending", {});
    }

    function approvePair(id, code) {
        if (!id || !code) {
            lastError = "pairing needs an id and the comparison code";
            return false;
        }
        return sendRequest("native.pair", { id: id, code: code });
    }

    function listPhoneDirectory(device, path) {
        if (!device || !device.id || !device.id.startsWith("native:")) {
            lastError = "native phone is unavailable";
            return false;
        }
        const id = device.id.substring("native:".length);
        return sendRequest("native.filesystem_list", { id: id, path: path || "." });
    }

    function refreshClipboardHistory() {
        return sendRequest("clipboard.history_list", {});
    }

    function pinClipboardHistory(id, pinned) {
        return sendRequest("clipboard.history_pin", { id: id, pinned: pinned });
    }

    function copyClipboardHistory(id) {
        return sendRequest("clipboard.history_copy", { id: id });
    }

    function clearClipboardHistory(includePinned) {
        return sendRequest("clipboard.history_clear", { include_pinned: !!includePinned });
    }

    function sendPhoneNotification(device, title, body) {
        if (!device || !device.id) {
            lastError = "native phone is unavailable";
            return false;
        }
        return sendRequest("notification.send", {
            device_id: device.id,
            app: "Handover",
            title: title,
            body: body
        });
    }

    function nativeAction(device, action) {
        if (!device || !device.id || !device.id.startsWith("native:"))
            return false;
        if (nativePending) {
            nativeNotice = "Another phone action is still pending";
            return false;
        }
        const id = device.id.substring("native:".length);
        if (!sendRequest("native." + action, { id: id })) {
            nativeNotice = "handoverd is disconnected";
            return false;
        }
        nativePending = true;
        nativeNotice = "";
        return true;
    }

    function sendClipboard(device, text) {
        if (!device || !device.id || !device.id.startsWith("native:")) {
            lastError = "native phone is unavailable";
            return false;
        }
        if (!text || text.length === 0 || text.length > 32768) {
            lastError = "clipboard text must be between 1 and 32 KiB";
            return false;
        }
        return sendRequest("clipboard.send", { device_id: device.id, text: text });
    }

    function sendCurrentClipboard(device) {
        if (!device || !device.id || !device.id.startsWith("native:")) {
            lastError = "native phone is unavailable";
            return false;
        }
        return sendRequest("clipboard.send_current", { device_id: device.id });
    }

    function remoteInput(device, action, deltaX, deltaY, button, text) {
        if (!device || !device.id || !device.id.startsWith("native:")) {
            lastError = "native phone is unavailable";
            return false;
        }
        return sendRequest("remote_input.send", {
            device_id: device.id,
            action: action,
            delta_x: Number(deltaX || 0),
            delta_y: Number(deltaY || 0),
            button: Number(button || 0),
            text: text || undefined
        });
    }

    function mediaCommand(session, action, value) {
        if (!session || !session.id || !action)
            return false;
        if (pendingCommand) {
            lastError = "another command is still pending";
            return false;
        }

        const fields = { id: session.id, action: action };
        if (action === "seek")
            fields.offset_ms = Number(value || 0);
        else if (action === "set_position")
            fields.position_ms = Number(value || 0);

        if (!sendRequest("media.control", fields)) {
            lastError = "handoverd is disconnected";
            return false;
        }
        pendingCommand = { method: "media.control", mediaId: session.id, action: action };
        lastError = "";
        return true;
    }

    function onConnected(socket) {
        retryTimer.stop();
        lastError = "";
        sendRequest("hello", null, socket);
        sendRequest("devices.list", null, socket);
        sendRequest("contacts.list", null, socket);
        sendRequest("native.pending", null, socket);
        sendRequest("custom.commands", null, socket);
        sendRequest("clipboard.mirror_status", null, socket);
        sendRequest("clipboard.history_list", null, socket);
        sendRequest("subscribe", { shares: true, media: true, messages: true }, socket);
    }

    function scheduleReconnect(message) {
        lastError = message;
        devices = [];
        notifications = [];
        mediaSessions = [];
        calls = [];
        contacts = [];
        callAudio = null;
        callNotice = "";
        lastReceivedShare = null;
        lastShareResult = null;
        lastShareProgress = null;
        filesystemResult = null;
        clipboardHistory = [];
        customCommands = [];
        customResult = null;
        pendingPeers = [];
        snapshotBuffer = null;
        messagingAccounts = [];
        conversations = [];
        conversationMessages = {};
        conversationCursors = {};
        typingStates = [];
        readStates = [];
        pairingPrompt = null;
        nativePending = false;
        nativeNotice = "";
        if (pendingCommand) {
            finishPending(false, message);
            pendingCommand = null;
        }
        if (pendingMessaging) {
            finishMessaging(false, message);
            pendingMessaging = null;
        }
        retryTimer.restart();
    }

    function replaceDevice(device) {
        const next = devices.filter(existing => existing.id !== device.id);
        next.push(device);
        devices = next;
    }

    function removeDevice(deviceId) {
        devices = devices.filter(device => device.id !== deviceId);
    }

    function sameNotificationId(first, second) {
        return first.device_id === second.device_id && first.local_id === second.local_id;
    }

    function replaceNotification(notification) {
        const next = notifications.filter(existing => !sameNotificationId(existing.id, notification.id));
        next.push(notification);
        notifications = next;
    }

    function removeNotification(notificationId) {
        notifications = notifications.filter(notification => !sameNotificationId(notification.id, notificationId));
    }

    function sameMediaId(first, second) {
        return first.device_id === second.device_id && first.player_id === second.player_id;
    }

    function replaceMediaSession(session) {
        const next = mediaSessions.filter(existing => !sameMediaId(existing.id, session.id));
        next.push(session);
        mediaSessions = next;
    }

    function removeMediaSession(mediaId) {
        mediaSessions = mediaSessions.filter(session => !sameMediaId(session.id, mediaId));
    }

    function applySnapshot(message, accumulating) {
        function concatField(name) {
            const incoming = message[name] || [];
            if (!accumulating || !snapshotBuffer)
                return incoming;
            return (snapshotBuffer[name] || []).concat(incoming);
        }
        const next = {
            devices: concatField("devices"),
            notifications: concatField("notifications"),
            media_sessions: concatField("media_sessions"),
            calls: concatField("calls"),
            messaging_accounts: concatField("messaging_accounts"),
            conversations: concatField("conversations"),
            typing_states: concatField("typing_states"),
            read_states: concatField("read_states")
        };
        if (accumulating && message.done !== true) {
            snapshotBuffer = next;
            return;
        }
        snapshotBuffer = null;
        devices = next.devices;
        notifications = next.notifications;
        mediaSessions = next.media_sessions;
        calls = next.calls;
        messagingAccounts = next.messaging_accounts;
        conversations = next.conversations;
        typingStates = next.typing_states;
        readStates = next.read_states;
        if (!message.conversations)
            refreshMessaging();
    }

    function finishPending(success, error) {
        if (!pendingCommand)
            return;
        if (pendingCommand.deviceId)
            shareFinished(pendingCommand.method, pendingCommand.deviceId, success, error);
        else if (pendingCommand.mediaId)
            mediaFinished(pendingCommand.action, pendingCommand.mediaId, success, error);
        else
            commandFinished(pendingCommand.method, pendingCommand.notificationId, success, error);
        pendingCommand = null;
    }

    function handleLine(line) {
        let message;
        try {
            message = JSON.parse(line);
        } catch (error) {
            console.warn("Handover: invalid JSON from daemon:", error);
            return;
        }

        if (message.protocol !== protocolVersion) {
            scheduleReconnect("unsupported daemon protocol " + message.protocol);
            return;
        }

        switch (message.type) {
        case "hello":
            if (!message.supported_protocols.includes(protocolVersion))
                scheduleReconnect("daemon does not support protocol " + protocolVersion);
            break;
        case "devices":
            devices = message.devices || [];
            break;
        case "notifications":
            notifications = message.notifications || [];
            break;
        case "media":
            mediaSessions = message.media_sessions || [];
            break;
        case "contacts":
            contacts = message.contacts || [];
            break;
        case "subscribed":
        case "snapshot":
            applySnapshot(message, false);
            break;
        case "subscribed_chunk":
        case "snapshot_chunk":
            applySnapshot(message, true);
            break;
        case "conversations_chunk":
            for (const conversation of message.conversations || [])
                replaceConversation(conversation);
            break;
        case "filesystem":
            filesystemResult = message.result || null;
            break;
        case "clipboard_mirror":
            clipboardMirrorEnabled = !!message.enabled;
            break;
        case "clipboard_history":
            clipboardHistory = message.entries || [];
            break;
        case "custom_commands":
            customCommands = message.commands || [];
            break;
        case "custom_result":
            customResult = message.result || null;
            break;
        case "native_pending":
            pendingPeers = message.pending || [];
            break;
        case "device_command_result":
            if (message.result) {
                nativePending = false;
                nativeNotice = message.result.accepted
                    ? "Phone accepted " + String(message.result.action || "command")
                    : "Phone rejected " + String(message.result.action || "command")
                        + (message.result.failure ? ": " + message.result.failure : "");
            }
            break;
        case "call_audio":
            callAudio = message.status;
            break;
        case "native_accepted":
            if (nativePending) {
                nativePending = false;
                nativeNotice = "Phone action accepted";
            }
            break;
        case "call_queued":
            callNotice = "Command queued; phone effect not confirmed";
            break;
        case "call_command_result":
            callNotice = message.result.accepted
                ? "Phone accepted " + message.result.action + "; call effect is not confirmed"
                : "Phone rejected " + message.result.action + ": "
                    + String(message.result.failure || "rejected").replaceAll("_", " ");
            break;
        case "call_updated":
            calls = calls.filter(call => call.device_id !== message.call.device_id).concat([message.call]);
            break;
        case "call_removed":
            calls = calls.filter(call => call.device_id !== message.device_id);
            break;
        case "device_added":
        case "device_updated":
            replaceDevice(message.device);
            break;
        case "device_removed":
            removeDevice(message.device_id);
            break;
        case "notification_added":
        case "notification_updated":
            replaceNotification(message.notification);
            break;
        case "notification_removed":
            removeNotification(message.notification_id);
            break;
        case "media_added":
        case "media_updated":
            replaceMediaSession(message.media_session);
            break;
        case "media_removed":
            removeMediaSession(message.media_session_id);
            break;
        case "share_received":
            lastReceivedShare = message.share;
            break;
        case "share_progress":
            lastShareProgress = message.progress;
            break;
        case "share_result":
            lastShareResult = message.result;
            break;
        case "share_accepted":
            if (pendingCommand && pendingCommand.deviceId === message.device_id)
                finishPending(true, "");
            break;
        case "accounts":
            break;
        case "conversations":
            for (const conversation of message.conversations || [])
                replaceConversation(conversation);
            break;
        case "history":
            replaceMessages(message.conversation_id, message.messages || [], message.cursor_next);
            {
                const key = conversationKey(message.conversation_id);
                if (exhaustiveHistoryLoads[key]) {
                    if (message.cursor_next) {
                        historyProgress(message.conversation_id, false,
                            (message.messages || []).length);
                        loadHistory(message.conversation_id, 100, message.cursor_next);
                    } else {
                        const loads = Object.assign({}, exhaustiveHistoryLoads);
                        delete loads[key];
                        exhaustiveHistoryLoads = loads;
                        historyProgress(message.conversation_id, true,
                            (message.messages || []).length);
                    }
                }
            }
            break;
        case "typing_states":
            typingStates = message.states || [];
            break;
        case "read_states":
            readStates = message.states || [];
            break;
        case "account_added":
        case "account_updated":
            {
                const next = messagingAccounts.filter(existing => existing.id !== message.account.id);
                next.push(message.account);
                messagingAccounts = next;
                if (message.account.authenticated)
                    loadConversations(message.account.id);
            }
            break;
        case "account_removed":
            messagingAccounts = messagingAccounts.filter(existing => existing.id !== message.account_id);
            conversations = conversations.filter(existing =>
                existing.id.account_id !== message.account_id);
            break;
        case "conversation_added":
        case "conversation_updated":
            replaceConversation(message.conversation);
            break;
        case "conversation_removed":
            removeConversation(message.conversation_id);
            break;
        case "message_added":
        case "message_updated":
            replaceMessages(message.message.id.conversation_id, [message.message]);
            break;
        case "message_removed":
            removeMessage(message.message_id);
            break;
        case "message_status":
            break;
        case "typing":
            replaceTyping(message.state);
            break;
        case "read_state":
            replaceReadState(message.state);
            break;
        case "pairing":
            pairingPrompt = { accountId: message.account_id, prompt: message.prompt };
            break;
        case "message_accepted":
            if (pendingMessaging)
                finishMessaging(true, "", message.request_id);
            break;
        case "conversation_accepted":
            if (pendingMessaging)
                finishMessaging(true, "", undefined,
                    message.conversation_id.account_id + ":" + message.conversation_id.local_id);
            break;
        case "account_accepted":
            if (pendingMessaging)
                finishMessaging(true, "", undefined, message.account_id);
            else
                refreshMessaging();
            break;
        case "media_accepted":
            if (pendingCommand && pendingCommand.mediaId
                    && sameMediaId(pendingCommand.mediaId, message.id))
                finishPending(true, "");
            break;
        case "command_completed":
            if (pendingCommand && pendingCommand.notificationId)
                finishPending(true, "");
            break;
        case "error":
            exhaustiveHistoryLoads = ({});
            lastError = message.code + ": " + message.message;
            console.warn("Handover:", lastError);
            finishPending(false, lastError);
            if (nativePending) {
                nativePending = false;
                nativeNotice = lastError;
            }
            if (pendingMessaging)
                finishMessaging(false, lastError);
            break;
        default:
            console.warn("Handover: unknown daemon message type", message.type);
        }
    }

    Timer {
        id: retryTimer
        interval: 2000
        repeat: false
        onTriggered: {
            socketLoader.active = false;
            recreateTimer.restart();
        }
    }

    Timer {
        id: recreateTimer
        interval: 1
        repeat: false
        onTriggered: socketLoader.active = true
    }

    LazyLoader {
        id: socketLoader
        active: root.socketPath.length > "/handover/handoverd.sock".length

        component: Component {
            Socket {
                id: socket
                property bool wasConnected: false

                path: root.socketPath
                connected: true
                parser: SplitParser {
                    onRead: data => root.handleLine(data)
                }

                onConnectionStateChanged: {
                    if (connected) {
                        wasConnected = true;
                        root.onConnected(socket);
                    } else if (wasConnected) {
                        root.scheduleReconnect("daemon disconnected");
                    }
                }
                onError: error => root.scheduleReconnect("socket error " + error)
            }
        }
    }
}
