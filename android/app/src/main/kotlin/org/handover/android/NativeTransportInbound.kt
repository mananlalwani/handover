package org.handover.android

import android.content.ComponentName
import android.media.RingtoneManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.VibrationEffect
import android.os.Vibrator
import android.net.Uri
import org.handover.android.NativeTransport.Companion.ACTION_PAIRED
import org.handover.android.NativeTransport.Companion.ACTION_PAIR_REQUEST
import org.handover.android.NativeTransport.Companion.ACTION_REMOTE_RESULT
import org.handover.android.NativeTransport.Companion.ACTION_REVOKED
import org.handover.android.NativeTransport.Companion.ACTION_SHARE_RECEIVED
import org.handover.android.NativeTransport.Companion.ACTION_TRANSFER_RESULT
import org.handover.android.NativeTransport.Companion.MAX_CLIPBOARD_FILE_BYTES
import org.handover.android.NativeTransport.Companion.MAX_FILE_BYTES
import org.handover.android.NativeTransport.Companion.PENDING_CODE_KEY
import org.handover.android.NativeTransport.Companion.PIN_KEY
import org.handover.android.NativeTransport.Companion.TRANSFER_INVALID_RESOURCE
import org.handover.android.NativeTransport.Companion.TRANSFER_REASONS
import org.handover.android.NativeTransport.Companion.TRANSFER_SIZE_LIMIT
import org.handover.android.NativeTransport.Companion.TRANSFER_STORAGE
import org.handover.android.NativeTransport.Companion.isPairingNonce
import org.handover.android.NativeTransport.Companion.isSafeBrowsePath
import org.handover.android.NativeTransport.Companion.isSha256Hex
import org.handover.android.NativeTransport.Companion.isTransferId
import org.handover.android.NativeTransport.Companion.isValidShareUrl
import org.handover.android.NativeTransport.Companion.pairingCode
import org.handover.android.NativeTransport.Companion.pairingCommitment
import org.handover.android.NativeTransport.Companion.safeFileName
import org.json.JSONObject
import java.io.BufferedInputStream

internal fun NativeTransport.handleInbound(message: JSONObject, input: BufferedInputStream) {
    if (message.optInt("protocol", -1) != 1) {
        socket?.close()
        return
    }
    when (message.optString("type")) {
        "ping" -> send(JSONObject().put("type", "pong").put("protocol", 1))
        "pong" -> Unit
        "filesystem_list" -> {
            if (serverFingerprint == null) return
            val requestId = message.optString("request_id")
            val path = message.optString("path", ".")
            if (!isTransferId(requestId)) return
            val response = if (!isSafeBrowsePath(path)) {
                JSONObject().put("type", "filesystem_failure").put("protocol", 1)
                    .put("request_id", requestId).put("reason", "unavailable")
            } else {
                runCatching { listPhoneDirectory(requestId, path) }.getOrElse {
                    JSONObject().put("type", "filesystem_failure").put("protocol", 1)
                        .put("request_id", requestId).put("reason", "unavailable")
                }
            }
            send(response)
        }
        "filesystem_entries", "filesystem_failure", "custom_command_list", "custom_command_result" -> {
            broadcast(ACTION_REMOTE_RESULT, message)
        }
        "hello" -> {
            val presentedFingerprint = peerFingerprint()
            val advertisedId = message.optString("id")
            if (presentedFingerprint == null || advertisedId != presentedFingerprint) {
                socket?.close()
                return
            }
            serverId = advertisedId.takeIf { it.isNotEmpty() }
            if (serverFingerprint != null) {
                approvalGranted = true
                return
            }
            // Unknown server: require a well-formed nonce commitment, then
            // reveal our own opening. The displayed code is derived after
            // both openings arrive, so it binds this ceremony rather than
            // only the long-lived certificates.
            val commit = message.optString("pair_commit")
            if (commit.isEmpty() || !isSha256Hex(commit)) {
                socket?.close()
                return
            }
            serverCommit = commit
            approvalGranted = false
            val nonce = ownNonce ?: return
            send(JSONObject().put("type", "pair_open").put("protocol", 1).put("nonce", nonce))
        }
        "pair_open" -> {
            if (serverFingerprint != null) return
            val commit = serverCommit
            val peer = serverId
            val nonce = ownNonce
            val serverNonce = message.optString("nonce")
            if (commit == null || peer == null || nonce == null
                || !isPairingNonce(serverNonce) || pairingCommitment(serverNonce) != commit
            ) {
                socket?.close()
                return
            }
            val code = pairingCode(identity.deviceId, nonce, peer, serverNonce)
            pendingCode = code
            preferences.edit().putString(PENDING_CODE_KEY, code).apply()
            broadcast(ACTION_PAIR_REQUEST, JSONObject().put("code", code).put("server_id", peer))
        }
        "paired" -> {
            if (!approvalGranted && serverFingerprint == null) return
            val fingerprint = peerFingerprint() ?: return
            preferences.edit().putString(PIN_KEY, fingerprint).apply()
            serverFingerprint = fingerprint
            preferences.edit().remove(PENDING_CODE_KEY).apply()
            broadcast(ACTION_PAIRED, JSONObject().put("server_id", serverId))
            sendBattery()
            publishConnectivity()
            // Proactively share the current notification list; the server
            // also requests it, so a lost frame is recovered on request.
            HandoverNotificationService.snapshotFor(context).let { (enabled, list) ->
                syncNotifications(enabled, list)
            }
            callObserver.refresh()
            // Same recovery for media sessions.
            MediaObserver.activePushSync()
            // The daemon drops keep-awake requests on disconnect, so a
            // still-enabled request is re-asserted on every session.
            if (desktopAwakeRequested()) requestDesktopAwake(true)
        }
        "revoke" -> {
            preferences.edit().remove(PIN_KEY).apply()
            serverFingerprint = null
            releaseWakeLock()
            broadcast(ACTION_REVOKED, JSONObject())
            socket?.close()
        }
        "battery_request" -> sendBattery()
        "ring" -> handleAudibleCommand(message, "ring")
        "user_ping" -> handleAudibleCommand(message, "ping")
        "keep_awake" -> {
            if (serverFingerprint == null) return
            val requestId = message.optString("request_id")
            if (!isTransferId(requestId)) return
            val inhibit = message.optBoolean("inhibit", false)
            if (setPhoneAwake(inhibit)) {
                sendDeviceCommandResult(requestId, "keep_awake", true, null)
            } else {
                sendDeviceCommandResult(requestId, "keep_awake", false, "unavailable")
            }
        }
        "tethering_settings" -> {
            if (serverFingerprint == null) return
            val requestId = message.optString("request_id")
            if (!isTransferId(requestId)) return
            // Third-party apps cannot toggle tethering directly; opening
            // the system screen for the user is the full extent of it.
            // There is no SDK constant for this action, so the documented
            // action string is used and failure is reported honestly.
            val opened = runCatching {
                context.startActivity(android.content.Intent(
                    "android.settings.TETHER_SETTINGS",
                ).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK))
                true
            }.getOrDefault(false)
            sendDeviceCommandResult(
                requestId,
                "tethering",
                opened,
                if (opened) null else "unavailable",
            )
        }
        "lock_device" -> {
            if (serverFingerprint == null) return
            val requestId = message.optString("request_id")
            if (!isTransferId(requestId)) return
            val admin = ComponentName(context, HandoverDeviceAdminReceiver::class.java)
            val manager = context.getSystemService(android.app.admin.DevicePolicyManager::class.java)
            if (!manager.isAdminActive(admin)) {
                sendDeviceCommandResult(requestId, "lock", false, "permission_denied")
            } else {
                runCatching { manager.lockNow() }
                    .onSuccess { sendDeviceCommandResult(requestId, "lock", true, null) }
                    .onFailure { sendDeviceCommandResult(requestId, "lock", false, "rejected") }
            }
        }
        "notifications_request" -> {
            if (serverFingerprint == null) return
            HandoverNotificationService.snapshotFor(context).let { (enabled, list) ->
                syncNotifications(enabled, list)
            }
        }
        "contacts_request" -> requestContactsSync()
        "clipboard_set" -> {
            if (serverFingerprint != null) {
                val requestId = message.optString("request_id")
                if (!isTransferId(requestId)) return
                val text = message.optString("text")
                val html = message.optString("html").takeIf { it.isNotEmpty() }
                val uri = message.optString("uri").takeIf { it.isNotEmpty() }
                if (text.toByteArray(Charsets.UTF_8).size > 32 * 1024) {
                    sendDeviceCommandResult(requestId, "clipboard", false, "rejected")
                    return
                }
                val previousRemoteHash = remoteClipboardHash
                remoteClipboardHash = clipboardHash(text)
                runCatching {
                    val clip = when {
                        html != null -> android.content.ClipData.newHtmlText("Handover", text, html)
                        else -> android.content.ClipData.newPlainText("Handover", text)
                    }
                    if (uri != null) {
                        clip.addItem(
                            context.contentResolver,
                            android.content.ClipData.Item(Uri.parse(uri)),
                        )
                    }
                    context.getSystemService(android.content.ClipboardManager::class.java)
                        .setPrimaryClip(clip)
                }.onSuccess {
                    sendDeviceCommandResult(requestId, "clipboard", true, null)
                }.onFailure {
                    remoteClipboardHash = previousRemoteHash
                    sendDeviceCommandResult(requestId, "clipboard", false, "rejected")
                }
            }
        }
        "notification_dismiss" -> {
            if (serverFingerprint == null) return
            HandoverNotificationService.dismiss(message.optString("key"))
        }
        "notification_reply" -> {
            if (serverFingerprint == null) return
            HandoverNotificationService.reply(message.optString("key"), message.optString("text"))
        }
        "notification_action" -> {
            if (serverFingerprint == null) return
            HandoverNotificationService.invokeAction(
                message.optString("key"), message.optString("action_id"),
            )
        }
        "remote_notification" -> {
            if (serverFingerprint == null) return
            val requestId = message.optString("request_id")
            if (!isTransferId(requestId)) return
            val posted = runCatching {
                HandoverNotificationService.postRemote(
                    requestId,
                    message.optString("app"),
                    message.optString("title"),
                    message.optString("body"),
                )
            }.getOrDefault(false)
            if (posted) {
                sendDeviceCommandResult(requestId, "notification", true, null)
            } else {
                sendDeviceCommandResult(requestId, "notification", false, "rejected")
            }
        }
        "call_request" -> {
            if (serverFingerprint == null) return
            callObserver.refresh()
        }
        "call_control" -> {
            if (serverFingerprint == null) return
            val requestId = message.optString("request_id")
            val action = message.optString("action")
            val requested = message.optLong("generation", -1L)
            val stale = requested != callObserver.generation
            val result = when (action) {
                "place" -> if (stale) CallController.Result(false, "stale_state")
                    else CallController.place(context, message.optString("address"))
                "answer" -> if (stale) CallController.Result(false, "stale_state")
                    else CallController.answer(context)
                "decline" -> if (stale) CallController.Result(false, "stale_state")
                    else CallController.hangup(context, decline = true)
                "hangup" -> if (stale) CallController.Result(false, "stale_state")
                    else CallController.hangup(context)
                else -> CallController.Result(false, "rejected")
            }
            send(JSONObject().put("type", "call_result").put("protocol", 1)
                .put("request_id", requestId).put("action", action)
                .put("accepted", result.accepted).apply {
                    result.failure?.let { put("failure", it) }
                })
            callObserver.refresh()
        }
        "media_request" -> {
            if (serverFingerprint == null) return
            MediaObserver.activePushSync()
        }
        "media_control" -> {
            if (serverFingerprint == null) return
            val requestId = message.optString("request_id")
            if (!isTransferId(requestId)) return
            val position = message.takeIf { it.has("position_ms") }?.optLong("position_ms")
            val applied = MediaObserver.executeControl(
                message.optString("player"), message.optString("action"), position,
            )
            sendDeviceCommandResult(
                requestId,
                "media",
                applied,
                if (applied) null else "rejected",
            )
            if (applied) {
                MediaObserver.activePushSync()
                Handler(Looper.getMainLooper()).postDelayed({
                    MediaObserver.activePushSync()
                }, 200)
            }
        }
        "share_result" -> {
            if (serverFingerprint == null) return
            val transferId = message.optString("transfer_id")
            if (!isTransferId(transferId)) return
            val status = message.optString("status")
            if (status != "completed" && status != "failed") return
            val reason = message.optString("reason").takeIf { it.isNotEmpty() }
            if ((status == "completed" && reason != null) ||
                (status == "failed" && reason !in TRANSFER_REASONS)) return
            completeTransfer(transferId, status, reason)
        }
        "clipboard_result" -> {
            if (serverFingerprint == null) return
            val transferId = message.optString("transfer_id")
            val status = message.optString("status")
            if (!isTransferId(transferId) || (status != "completed" && status != "failed")) return
            val reason = message.optString("reason").takeIf { it.isNotEmpty() }
            broadcast(ACTION_TRANSFER_RESULT, JSONObject().put("transfer_id", transferId)
                .put("status", status).apply { reason?.let { put("reason", it) } })
        }
        "share_url" -> {
            if (serverFingerprint == null) return
            val transferId = message.optString("transfer_id")
            val url = message.optString("url")
            if (!isTransferId(transferId) || !isValidShareUrl(url)) {
                sendTransferResult(transferId, "failed", TRANSFER_INVALID_RESOURCE)
                return
            }
            runCatching {
                notifyReceived("url", url, url)
                TransferHistory.add(context, "url", url, Uri.parse(url))
                broadcast(ACTION_SHARE_RECEIVED, JSONObject().put("kind", "url").put("url", url)
                    .put("source", serverId ?: serverFingerprint))
            }.onSuccess { sendTransferResult(transferId, "completed", null) }
                .onFailure { sendTransferResult(transferId, "failed", TRANSFER_STORAGE) }
        }
        "share_file" -> {
            if (serverFingerprint == null) return
            val transferId = message.optString("transfer_id")
            val name = safeFileName(message.optString("name")) ?: run {
                sendTransferFailureAndClose(transferId, TRANSFER_INVALID_RESOURCE); return
            }
            val size = message.optLong("size", -1L)
            if (size !in 0..MAX_FILE_BYTES) {
                sendTransferFailureAndClose(transferId, TRANSFER_SIZE_LIMIT); return
            }
            if (!isTransferId(transferId)) { socket?.close(); return }
            if (message.optBoolean("clipboard", false)) {
                val mime = message.optString("mime").takeIf { it.isNotEmpty() }
                if (mime == null || size > MAX_CLIPBOARD_FILE_BYTES) {
                    sendTransferFailureAndClose(transferId, TRANSFER_INVALID_RESOURCE); return
                }
                receiveClipboardFile(input, transferId, name, size, mime)
            } else {
                receiveFile(input, transferId, name, size)
            }
        }
    }
}

internal fun NativeTransport.ringPhone() {
    if (serverFingerprint == null) return
    val ringtone = RingtoneManager.getRingtone(
        context,
        RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE),
    )
    ringtone?.play()
    Handler(Looper.getMainLooper()).postDelayed({ ringtone?.stop() }, 4_000)
    val vibrator = context.getSystemService(Vibrator::class.java)
    if (vibrator?.hasVibrator() == true) {
        val effect = VibrationEffect.createOneShot(1200, VibrationEffect.DEFAULT_AMPLITUDE)
        vibrator.vibrate(effect)
    }
}

internal fun NativeTransport.handleAudibleCommand(message: JSONObject, action: String) {
    if (serverFingerprint == null) return
    val requestId = message.optString("request_id")
    if (!isTransferId(requestId)) return
    runCatching { ringPhone() }
        .onSuccess { sendDeviceCommandResult(requestId, action, true, null) }
        .onFailure { sendDeviceCommandResult(requestId, action, false, "unavailable") }
}

