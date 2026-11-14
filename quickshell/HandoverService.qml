pragma Singleton

import QtQml
import Quickshell
import Quickshell.Io

Singleton {
    id: root

    readonly property int protocolVersion: 1
    readonly property int maxLineBytes: 2 * 1024 * 1024
    readonly property int maxSnapshotBytes: 16 * 1024 * 1024
    readonly property int maxSnapshotChunks: 256
    readonly property string socketPath: String(Quickshell.env("XDG_RUNTIME_DIR") || "")
        + "/handover/handoverd.sock"
    readonly property bool connected: socketLoader.item
        ? socketLoader.item.connected
        : false
    property var archiveSettings: null
    property bool archiveSaving: false
    signal archiveSettingsFinished(bool saving, bool success, string error)
    property var devices: []
    property var notifications: []
    property var mediaSessions: []
    property var calls: []
    property var contacts: []
    property string contactsSyncError: ""
    property var callAudio: null
    property string callNotice: ""
    function refreshCallAudio() {
        callAudio = null;
        sendRequest("calls.audio", {});
    }
    function refreshAudioControls() {
        return sendRequest("audio.controls", {});
    }
    function configureAudioRouting(change) {
        return sendRequest("audio.configure", { change: change });
    }
    function refreshContacts() {
        return sendRequest("contacts.list", {});
    }

    function syncContacts(device) {
        if (!device || !device.id || !device.id.startsWith("native:")) {
            lastError = "native phone is unavailable";
            return false;
        }
        contactsSyncError = "";
        return sendRequest("contacts.sync", { device_id: device.id });
    }

    function contactPhoneKey(phone) {
        const digits = String(phone || "").replace(/[^0-9]/g, "");
        // NANP national numbers have ten digits. Only the country-code 1
        // equivalent is accepted; arbitrary suffix matches can join people
        // from different countries or mistake short codes for contacts.
        const national = digits.length === 11 && digits.startsWith("1")
            ? digits.slice(1) : digits;
        if ((digits.length === 10 || (digits.length === 11 && digits.startsWith("1")))
            && /^[2-9][0-9]{2}[2-9][0-9]{6}$/.test(national))
            return "nanp:" + national;
        return digits ? "digits:" + digits : "";
    }

    function contactForParticipant(participant) {
        if (!participant)
            return null;
        const address = String(participant.address || "").trim();
        if (!address)
            return null;
        const email = address.toLowerCase();
        const key = contactPhoneKey(address);
        const matches = contacts.filter(contact => email.includes("@")
            ? (contact.emails || []).some(candidate =>
                String(candidate).trim().toLowerCase() === email)
            : (key.length > 0 && (contact.phones || []).some(phone =>
                contactPhoneKey(phone) === key)));
        return matches.length === 1 ? matches[0] : null;
    }

    function contactLabel(participant) {
        if (participant.is_self)
            return "You";
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
    property var audioStreams: []
    property var audioControls: null
    property var snapshotBuffer: null
    property var listAssembly: null
    property string snapshotKind: ""
    property int snapshotBytes: 0
    property int snapshotChunks: 0
    property string lastError: ""
    property string nativeNotice: ""
    property bool nativePending: false
    property var pendingCommand: null
    readonly property int maxRequests: 16
    property var activeRequests: []
    property var pendingNative: null
    signal commandFinished(string method, var notificationId, bool success, string error)
    signal shareFinished(string method, string deviceId, bool success, string error, bool unknown)
    signal shareAccepted(string method, string deviceId, string transferId)
    signal shareCancelFinished(string deviceId, string transferId, bool cancelled, string error, bool unknown)
    signal sharesInterrupted()
    signal mediaFinished(string action, var mediaId, bool success, string error)
    // Messaging state. Conversations belong to messaging accounts, never to
    // physical devices. Message windows are keyed by "account:thread".
    property var messagingAccounts: []
    property var conversations: []
    property var conversationMessages: ({})
    property var conversationCursors: ({})
    property var exhaustiveHistoryLoads: ({})
    property var historyRequests: ({})
    property var historyFocus: null
    property var automaticHistoryNext: null
    function setHistoryFocus(conversationId) {
        if (historyFocus && (!conversationId || !sameConversationId(historyFocus, conversationId))) {
            const key = conversationKey(historyFocus);
            if (exhaustiveHistoryLoads[key] && exhaustiveHistoryLoads[key].automatic) {
                const loads = Object.assign({}, exhaustiveHistoryLoads);
                delete loads[key];
                exhaustiveHistoryLoads = loads;
            }
            automaticHistoryNext = null;
        }
        historyFocus = conversationId;
    }
    Timer {
        id: automaticHistoryPageTimer
        interval: 1000
        onTriggered: {
            const next = root.automaticHistoryNext;
            root.automaticHistoryNext = null;
            if (next && root.historyFocus && root.sameConversationId(root.historyFocus, next.id)
                    && root.isLoadingAllHistory(next.id))
                if (!root.loadHistory(next.id, 100, next.cursor)) {
                    root.automaticHistoryNext = next;
                    automaticHistoryPageTimer.restart();
                }
        }
    }
    property var typingStates: []
    property var readStates: []
    property var outgoingOperations: []
    property var pairingPrompt: null
    property var pendingMessaging: null
    signal messagingFinished(string method, string requestId, bool success, string error)
    signal messagingAccepted(string method, string subject, bool success, string error)
    signal historyProgress(var conversationId, bool complete, int messageCount)

    function conversationKey(id) {
        return id.account_id + ":" + id.local_id;
    }

    function dropAccountEntries(entries, accountId) {
        const prefix = accountId + ":";
        const next = Object.assign({}, entries);
        for (const key of Object.keys(next)) {
            if (key.startsWith(prefix))
                delete next[key];
        }
        return next;
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
        const command = { method: method };
        if (!sendRequest(method, fields, null, command)) {
            return false;
        }
        pendingMessaging = command;
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

    function loadOutgoingOperations() {
        outgoingOperations = [];
        sendRequest("messages.outgoing", null);
    }

    function replaceOutgoingOperation(operation) {
        const next = outgoingOperations.filter(existing => existing.id !== operation.id);
        next.push(operation);
        outgoingOperations = next;
    }

    function stopHistoryLoad(conversationId) {
        const key = conversationKey(conversationId);
        const requests = Object.assign({}, historyRequests);
        delete requests[key];
        historyRequests = requests;
        const loads = Object.assign({}, exhaustiveHistoryLoads);
        delete loads[key];
        exhaustiveHistoryLoads = loads;
        if (automaticHistoryNext && sameConversationId(automaticHistoryNext.id, conversationId))
            automaticHistoryNext = null;
    }

    function loadHistory(conversationId, limit, cursor) {
        const key = conversationKey(conversationId);
        if (historyRequests[key])
            return false;
        const fields = { conversation_id: conversationId };
        if (limit)
            fields.limit = limit;
        if (cursor)
            fields.cursor = cursor;
        const sent = sendRequest("messages.history", fields);
        if (sent) {
            const requests = Object.assign({}, historyRequests);
            requests[key] = true;
            historyRequests = requests;
        }
        return sent;
    }

    function loadAllHistory(conversationId, automatic) {
        if (historyRequests[conversationKey(conversationId)])
            return false;
        const key = conversationKey(conversationId);
        const loads = Object.assign({}, exhaustiveHistoryLoads);
        loads[key] = { automatic: !!automatic, lastCursor: null };
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
        if (!conversationId)
            return false;
        const account = messagingAccounts.find(item => item.id === conversationId.account_id);
        const conversation = conversations.find(item => sameConversationId(item.id, conversationId));
        if (!account || !account.connected || !account.authenticated
                || !conversation || !conversation.capabilities
                || !conversation.capabilities.includes("read_receipts"))
            return false;
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

    function sendRequest(method, fields, socketOverride, owner) {
        const encoded = JSON.stringify(Object.assign({ protocol: protocolVersion, method: method }, fields || {})) + "\n";
        if (socketOverride) {
            if (!socketOverride.connected)
                return false;
            socketOverride.write(encoded);
            socketOverride.flush();
            return true;
        }
        if (!connected) {
            lastError = "handoverd is disconnected";
            return false;
        }
        if (method === "messages.archive_settings" && fields && fields.limit_mib !== undefined && archiveSaving) {
            lastError = "another archive update is still pending";
            return false;
        }
        // A newer refresh supersedes an older read of the same collection.
        // Mutations and paged history are never cancelled or queued here.
        const refreshes = ["contacts.list", "calls.audio", "custom.commands", "native.pending",
            "clipboard.history_list", "clipboard.mirror_status", "messages.conversations"];
        const refresh = refreshes.includes(method)
            || (method === "messages.archive_settings" && (!fields || fields.limit_mib === undefined));
        if (refresh) {
            for (const previous of activeRequests.slice()) {
                if (previous.method === method && !previous.owner
                        && previous.fields.account_id === (fields || {}).account_id
                        && previous.fields.limit_mib === undefined)
                    finishRequest(previous);
            }
        }
        if (activeRequests.length >= maxRequests) {
            lastError = "too many requests are still pending";
            return false;
        }
        if (utf8ByteLength(encoded) > maxLineBytes) {
            lastError = "request exceeds the IPC size limit";
            return false;
        }
        const transport = requestComponent.createObject(root, { socketPath: socketPath, encodedRequest: encoded });
        const request = { method: method, fields: fields || {}, owner: owner, transport: transport,
            listAssembly: null, done: false };
        activeRequests = activeRequests.concat([request]);
        if (method === "messages.archive_settings" && fields && fields.limit_mib !== undefined)
            archiveSaving = true;
        transport.received.connect(line => handleLine(line, request));
        transport.failed.connect((reason, submitted) => failRequest(request, reason, submitted));
        // Install callbacks and pending UI ownership before a local socket can connect.
        Qt.callLater(() => transport.start());
        return true;
    }

    function finishRequest(request) {
        if (!request || request.done)
            return;
        request.done = true;
        request.listAssembly = null;
        activeRequests = activeRequests.filter(item => item !== request);
        request.transport.close();
        Qt.callLater(() => request.transport.destroy());
    }

    function failRequest(request, reason, submitted) {
        if (request.done)
            return;
        const error = submitted ? reason + ". Outcome is unknown; check current state before trying again." : reason;
        lastError = error;
        if (request.method === "messages.history" && request.fields.conversation_id)
            stopHistoryLoad(request.fields.conversation_id);
        if (request.method === "messages.archive_settings") {
            if (request.fields.limit_mib !== undefined)
                archiveSaving = false;
            archiveSettingsFinished(request.fields.limit_mib !== undefined, false, error);
        }
        if (request.owner && request.owner === pendingCommand)
            finishPending(false, error, submitted);
        if (request.owner && request.owner === pendingMessaging)
            finishMessaging(false, error);
        if (request.owner && request.owner === pendingNative) {
            pendingNative = null;
            nativePending = false;
            nativeNotice = error;
        }
        finishRequest(request);
    }

    function abortRequests(reason) {
        for (const request of activeRequests.slice())
            failRequest(request, reason, request.transport.submitted);
    }

    function sendCommand(method, fields) {
        if (pendingCommand) {
            lastError = "another notification command is still pending";
            return false;
        }
        const command = { method: method, notificationId: fields.notification_id };
        if (!sendRequest(method, fields, null, command)) {
            return false;
        }
        pendingCommand = command;
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
        const command = { method: method, deviceId: device.id, transferId: fields.transfer_id || "" };
        if (!sendRequest(method, Object.assign({ device_id: device.id }, fields), null, command)) {
            return false;
        }
        pendingCommand = command;
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
        const command = { method: "native." + action };
        if (!sendRequest(command.method, { id: id }, null, command)) {
            nativeNotice = lastError;
            return false;
        }
        pendingNative = command;
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

        const command = { method: "media.control", mediaId: session.id, action: action };
        if (!sendRequest("media.control", fields, null, command)) {
            return false;
        }
        pendingCommand = command;
        lastError = "";
        return true;
    }

    function onConnected(socket) {
        retryTimer.stop();
        lastError = "";
        sendRequest("hello", null, socket);
        sendRequest("subscribe", { shares: true, media: true, messages: true }, socket);
        sendRequest("audio.subscribe", {}, socket);
        // The root's connected binding updates after the socket's signal handler.
        Qt.callLater(() => {
            if (socketLoader.item !== socket || !socket.connected)
                return;
            sendRequest("contacts.list", null);
            sendRequest("messages.archive_settings", {});
            sendRequest("native.pending", null);
            sendRequest("custom.commands", null);
            sendRequest("clipboard.mirror_status", null);
            sendRequest("clipboard.history_list", null);
            refreshAudioControls();
            // Outgoing operations follow the subscription snapshot on that socket.
        });
    }

    function scheduleReconnect(message) {
        sharesInterrupted();
        const hadNativeRequest = !!pendingNative;
        abortRequests(message);
        lastError = message;
        audioStreams = [];
        audioControls = null;
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
        clearSnapshotAssembly();
        listAssembly = null;
        messagingAccounts = [];
        conversations = [];
        conversationMessages = {};
        exhaustiveHistoryLoads = ({});
        historyRequests = ({});
        automaticHistoryNext = null;
        conversationCursors = {};
        typingStates = [];
        readStates = [];
        outgoingOperations = [];
        pairingPrompt = null;
        nativePending = false;
        pendingNative = null;
        if (!hadNativeRequest)
            nativeNotice = "";
        if (pendingCommand) {
            finishPending(false, message, true);
            pendingCommand = null;
        }
        if (pendingMessaging) {
            finishMessaging(false, message);
            pendingMessaging = null;
        }
        retryTimer.restart();
    }

    function clearSnapshotAssembly() {
        snapshotBuffer = null;
        snapshotKind = "";
        snapshotBytes = 0;
        snapshotChunks = 0;
    }

    // Count UTF-8 bytes without allocating an encoded byte array.
    function utf8ByteLength(value) {
        let bytes = 0;
        for (let index = 0; index < value.length; index++) {
            const unit = value.charCodeAt(index);
            if (unit < 0x80) {
                bytes++;
            } else if (unit < 0x800) {
                bytes += 2;
            } else if (unit >= 0xd800 && unit <= 0xdbff
                    && index + 1 < value.length) {
                const next = value.charCodeAt(index + 1);
                if (next >= 0xdc00 && next <= 0xdfff) {
                    bytes += 4;
                    index++;
                } else {
                    bytes += 3;
                }
            } else {
                bytes += 3;
            }
            if (bytes > maxLineBytes)
                return bytes;
        }
        return bytes;
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
        clearSnapshotAssembly();
        outgoingOperations = [];
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

    function finishPending(success, error, unknown) {
        if (!pendingCommand)
            return;
        if (pendingCommand.method === "share.cancel")
            shareCancelFinished(pendingCommand.deviceId, pendingCommand.transferId, false, error, !!unknown);
        else if (pendingCommand.deviceId)
            shareFinished(pendingCommand.method, pendingCommand.deviceId, success, error, !!unknown);
        else if (pendingCommand.mediaId)
            mediaFinished(pendingCommand.action, pendingCommand.mediaId, success, error);
        else
            commandFinished(pendingCommand.method, pendingCommand.notificationId, success, error);
        pendingCommand = null;
    }

    function expectedReply(method) {
        const replies = {
            "hello": "hello", "devices.list": "devices", "notifications.list": "notifications",
            "contacts.list": "contacts", "media.list": "media", "calls.list": "calls",
            "calls.audio": "call_audio", "calls.control": "call_queued", "native.call": "call_queued",
            "native.pending": "native_pending", "native.peers": "native_peers",
            "clipboard.mirror_status": "clipboard_mirror", "clipboard.history_list": "clipboard_history",
            "custom.commands": "custom_commands", "custom.run": "custom_result",
            "audio.list": "audio_streams", "audio.controls": "audio_controls",
            "audio.configure": "audio_controls", "notification.dismiss": "command_completed",
            "notification.action": "command_completed", "notification.reply": "command_completed",
            "media.control": "media_accepted", "share.file": "share_accepted", "share.url": "share_accepted",
            "share.cancel": "share_cancelled", "messages.accounts": "accounts",
            "messages.conversations": "conversations", "messages.history": "history",
            "messages.archive_settings": "archive_settings", "messages.outgoing": "outgoing_operations",
            "messages.typing_states": "typing_states", "messages.read_states": "read_states",
            "messages.send": "message_accepted", "messages.send_file": "message_accepted",
            "messages.react": "message_accepted", "messages.unreact": "message_accepted",
            "messages.delete": "message_accepted", "messages.open": "message_accepted",
            "messages.read": "conversation_accepted", "messages.typing": "conversation_accepted",
            "messages.login": "account_accepted", "messages.logout": "account_accepted",
            "messages.sync": "account_accepted"
        };
        return Object.prototype.hasOwnProperty.call(replies, method) ? replies[method] : "native_accepted";
    }

    function handleLine(line, request) {
        if (request && request.done)
            return;
        const reject = reason => {
            if (request)
                failRequest(request, reason, request.transport.submitted);
            else
                scheduleReconnect(reason);
        };
        // SplitParser calls this after a newline; this caps parsed lines and
        // retained snapshots, not its own pre-newline buffer.
        const lineBytes = utf8ByteLength(line);
        if (lineBytes > maxLineBytes) {
            reject("daemon message exceeds protocol limit");
            return;
        }
        let message;
        try {
            message = JSON.parse(line);
        } catch (error) {
            reject("daemon sent invalid JSON");
            return;
        }

        if (!message || typeof message !== "object" || Array.isArray(message)
                || typeof message.type !== "string") {
            reject("daemon sent invalid message");
            return;
        }

        if (message.protocol !== protocolVersion) {
            reject("unsupported daemon protocol");
            return;
        }

        const ownsCommand = !!request && !!request.owner && request.owner === pendingCommand;
        const ownsMessaging = !!request && !!request.owner && request.owner === pendingMessaging;
        const ownsNative = !!request && !!request.owner && request.owner === pendingNative;
        if (request && message.type !== "error") {
            const expected = expectedReply(request.method);
            if (message.type !== expected && message.type !== expected + "_chunk") {
                reject("daemon response does not match the request");
                return;
            }
            if ((message.type === "history" || message.type === "conversation_accepted")
                    && (!request.fields.conversation_id || !message.conversation_id
                        || !sameConversationId(message.conversation_id, request.fields.conversation_id))) {
                reject("daemon response names another conversation");
                return;
            }
            if (message.type === "share_accepted" && message.device_id !== request.fields.device_id) {
                reject("daemon response names another device");
                return;
            }
            if (message.type === "media_accepted" && (!request.fields.id || !message.id
                    || !sameMediaId(message.id, request.fields.id))) {
                reject("daemon response names another media session");
                return;
            }
            if (message.type === "command_completed" && (!request.fields.notification_id || !message.notification_id
                    || !sameNotificationId(message.notification_id, request.fields.notification_id))) {
                reject("daemon response names another notification");
                return;
            }
            if (message.type === "account_accepted" && message.account_id !== request.fields.account_id) {
                reject("daemon response names another account");
                return;
            }
            if (message.type === "message_accepted" && (typeof message.request_id !== "string" || !message.request_id)) {
                reject("daemon acceptance has no operation identifier");
                return;
            }
        }

        const isSnapshotChunk = message.type === "subscribed_chunk"
            || message.type === "snapshot_chunk";
        const listFields = {
            notifications_chunk: "notifications", media_chunk: "media_sessions",
            contacts_chunk: "contacts", conversations_chunk: "conversations",
            outgoing_operations: "operations"
        };
        const isListChunk = Object.prototype.hasOwnProperty.call(listFields, message.type);
        const currentList = request ? request.listAssembly : listAssembly;
        if (!request && snapshotBuffer && (!isSnapshotChunk || message.type !== snapshotKind)) {
            reject("daemon interrupted snapshot sequence");
            return;
        }
        if (currentList && (!isListChunk || message.type !== currentList.kind)) {
            reject("daemon interrupted list sequence");
            return;
        }
        if (isSnapshotChunk && currentList) {
            reject("daemon interrupted list sequence");
            return;
        }
        if (!request && isListChunk && snapshotBuffer) {
            reject("daemon interrupted snapshot sequence");
            return;
        }
        if (isListChunk) {
            const field = listFields[message.type];
            if (!Array.isArray(message[field]) || typeof message.done !== "boolean") {
                reject("daemon sent invalid list chunk");
                return;
            }
            const nextBytes = (currentList ? currentList.bytes : 0) + lineBytes;
            const nextChunks = (currentList ? currentList.chunks : 0) + 1;
            if (nextBytes > maxSnapshotBytes || nextChunks > maxSnapshotChunks) {
                reject("daemon list exceeds protocol limit");
                return;
            }
            const items = (currentList ? currentList.items : []).concat(message[field]);
            if (message.done) {
                if (message.type === "notifications_chunk")
                    notifications = items;
                else if (message.type === "media_chunk")
                    mediaSessions = items;
                else if (message.type === "contacts_chunk")
                    contacts = items;
                else if (message.type === "conversations_chunk") {
                    for (const conversation of items)
                        replaceConversation(conversation);
                } else
                    outgoingOperations = items;
                if (request)
                    finishRequest(request);
                else
                    listAssembly = null;
            } else {
                const next = { kind: message.type, items, bytes: nextBytes, chunks: nextChunks };
                if (request)
                    request.listAssembly = next;
                else
                    listAssembly = next;
            }
            return;
        }
        if (isSnapshotChunk) {
            const kind = message.type;
            if (!snapshotBuffer)
                snapshotKind = kind;
            const nextBytes = snapshotBytes + lineBytes;
            if (nextBytes > maxSnapshotBytes || snapshotChunks >= maxSnapshotChunks) {
                reject("daemon snapshot exceeds protocol limit");
                return;
            }
            snapshotBytes = nextBytes;
            snapshotChunks++;
        }

        switch (message.type) {
        case "hello":
            if (!Array.isArray(message.supported_protocols) || !message.supported_protocols.includes(protocolVersion))
                reject("daemon does not support this protocol");
            break;
        case "devices":
            devices = message.devices || [];
            break;
        case "notifications":
            notifications = message.notifications || [];
            break;
        case "audio_streams":
            audioStreams = (message.streams || []).map(incoming => {
                const previous = audioStreams.find(status => status.device_id === incoming.device_id);
                return previous && Number(previous.revision || 0) > Number(incoming.revision || 0) ? previous : incoming;
            });
            break;
        case "audio_stream_updated":
            const previousAudio = audioStreams.find(s => s.device_id === message.status.device_id);
            if (!previousAudio || Number(message.status.revision || 0) >= Number(previousAudio.revision || 0))
                audioStreams = audioStreams.filter(s => s.device_id !== message.status.device_id).concat([message.status]);
            break;
        case "audio_controls":
        case "audio_controls_updated":
            if (!message.snapshot || !message.snapshot.settings)
                reject("daemon sent invalid laptop audio controls");
            else if (!audioControls || Number(message.snapshot.revision || 0) >= Number(audioControls.revision || 0))
                audioControls = message.snapshot;
            break;
        case "media":
            mediaSessions = message.media_sessions || [];
            break;
        case "contacts":
            contacts = message.contacts || [];
            break;
        case "contacts_synced":
            contactsSyncError = "";
            refreshContacts();
            break;
        case "contacts_removed":
            contacts = contacts.filter(contact => contact.device_id !== message.device_id);
            break;
        case "contacts_sync_failed":
            switch (message.failure) {
            case "permission_denied":
                contactsSyncError = "Contacts permission denied";
                break;
            case "too_large":
                contactsSyncError = "Phone contact list is too large";
                break;
            case "interrupted":
                contactsSyncError = "Contacts sync interrupted";
                break;
            case "rejected":
                contactsSyncError = "Phone rejected contacts sync";
                break;
            default:
                contactsSyncError = "Contacts sync failed";
            }
            break;
        case "subscribed":
        case "snapshot":
            applySnapshot(message, false);
            if (message.type === "snapshot")
                refreshContacts();
            break;
        case "subscribed_chunk":
        case "snapshot_chunk":
            applySnapshot(message, true);
            if (message.type === "snapshot_chunk" && message.done === true)
                refreshContacts();
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
            if (ownsCommand && (pendingCommand.method === "audio.start" || pendingCommand.method === "audio.stop"))
                finishPending(true, "");
            if (ownsNative) {
                pendingNative = null;
                nativePending = false;
                nativeNotice = "Command queued; phone effect not confirmed";
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
            if (ownsCommand && pendingCommand.deviceId === message.device_id
                    && (pendingCommand.method === "share.file" || pendingCommand.method === "share.url")) {
                shareAccepted(pendingCommand.method, message.device_id, message.transfer_id || "");
                finishPending(true, "");
            }
            break;
        case "share_cancelled":
            if (typeof message.cancelled !== "boolean") {
                reject("daemon sent invalid cancellation result");
                return;
            }
            if (ownsCommand && pendingCommand.method === "share.cancel") {
                const command = pendingCommand;
                pendingCommand = null;
                shareCancelFinished(command.deviceId, command.transferId, message.cancelled, "", false);
            }
            break;
        case "accounts":
            break;
        case "conversations":
            for (const conversation of message.conversations || [])
                replaceConversation(conversation);
            break;
        case "archive_settings":
            archiveSettings = { limitMiB: message.limit_mib, usedBytes: message.used_bytes };
            if (request) {
                if (request.fields.limit_mib !== undefined)
                    archiveSaving = false;
                archiveSettingsFinished(request.fields.limit_mib !== undefined, true, "");
            }
            break;
        case "history":
            {
                const requests = Object.assign({}, historyRequests);
                delete requests[conversationKey(message.conversation_id)];
                historyRequests = requests;
            }
            replaceMessages(message.conversation_id, message.messages || [], message.cursor_next);
            {
                const key = conversationKey(message.conversation_id);
                if (exhaustiveHistoryLoads[key]) {
                    if (message.cursor_next) {
                        historyProgress(message.conversation_id, false,
                            (message.messages || []).length);
                        const load = exhaustiveHistoryLoads[key];
                        if (load.lastCursor === message.cursor_next) {
                            const loads = Object.assign({}, exhaustiveHistoryLoads);
                            delete loads[key];
                            exhaustiveHistoryLoads = loads;
                            lastError = "History cursor did not advance";
                        } else {
                            load.lastCursor = message.cursor_next;
                            if (load.automatic) {
                                automaticHistoryNext = { id: message.conversation_id, cursor: message.cursor_next };
                                automaticHistoryPageTimer.restart();
                            } else
                                loadHistory(message.conversation_id, 100, message.cursor_next);
                        }
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
                // Conversations are authoritative subscription state, not a second snapshot.
            }
            break;
        case "account_removed":
            messagingAccounts = messagingAccounts.filter(existing => existing.id !== message.account_id);
            conversations = conversations.filter(existing =>
                existing.id.account_id !== message.account_id);
            outgoingOperations = outgoingOperations.filter(operation =>
                operation.conversation_id.account_id !== message.account_id);
            conversationMessages = dropAccountEntries(conversationMessages, message.account_id);
            conversationCursors = dropAccountEntries(conversationCursors, message.account_id);
            historyRequests = dropAccountEntries(historyRequests, message.account_id);
            exhaustiveHistoryLoads = dropAccountEntries(exhaustiveHistoryLoads, message.account_id);
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
        case "outgoing_operation":
            replaceOutgoingOperation(message.operation);
            break;
        case "outgoing_operation_removed":
            outgoingOperations = outgoingOperations.filter(
                operation => operation.id !== message.operation_id);
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
            if (ownsMessaging)
                finishMessaging(true, "", message.request_id);
            break;
        case "conversation_accepted":
            if (ownsMessaging)
                finishMessaging(true, "", undefined,
                    message.conversation_id.account_id + ":" + message.conversation_id.local_id);
            break;
        case "account_accepted":
            if (ownsMessaging)
                finishMessaging(true, "", undefined, message.account_id);
            else
                refreshMessaging();
            break;
        case "media_accepted":
            if (ownsCommand && pendingCommand.mediaId
                    && sameMediaId(pendingCommand.mediaId, message.id))
                finishPending(true, "");
            break;
        case "command_completed":
            if (ownsCommand && pendingCommand.notificationId)
                finishPending(true, "");
            break;
        case "error":
            // Stop the history load this error names. An error for another
            // conversation, or one with no conversation, leaves other loads running.
            if (request && request.method === "messages.history")
                stopHistoryLoad(request.fields.conversation_id);
            lastError = message.code + ": " + message.message;
            console.warn("Handover:", lastError);
            if (request && request.method === "messages.archive_settings") {
                if (request.fields.limit_mib !== undefined)
                    archiveSaving = false;
                archiveSettingsFinished(request.fields.limit_mib !== undefined, false, lastError);
            }
            if (ownsCommand)
                finishPending(false, lastError);
            if (ownsNative) {
                pendingNative = null;
                nativePending = false;
                nativeNotice = lastError;
            }
            if (ownsMessaging)
                finishMessaging(false, lastError);
            break;
        default:
            console.warn("Handover: unknown daemon message type");
        }
        if (request) {
            if ((ownsCommand && pendingCommand === request.owner)
                    || (ownsMessaging && pendingMessaging === request.owner))
                failRequest(request, "daemon response did not resolve the command", request.transport.submitted);
            else
                finishRequest(request);
        }
    }

    Component {
        id: requestComponent
        IpcRequest { }
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
