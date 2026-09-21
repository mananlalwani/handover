package org.handover.android

import android.content.Intent
import android.net.Uri
import android.os.Build
import org.json.JSONObject
import java.security.MessageDigest
import java.util.UUID
import org.handover.android.NativeTransport.Companion.CLIPBOARD_SYNC_KEY
import org.handover.android.NativeTransport.Companion.OVERLAY_ASSIST_KEY
import org.handover.android.NativeTransport.Companion.STREAM_BUFFER_BYTES
import org.handover.android.NativeTransport.Companion.fileMetadata

fun NativeTransport.setClipboardSync(enabled: Boolean) {
    preferences.edit().putBoolean(CLIPBOARD_SYNC_KEY, enabled).apply()
    configureClipboardSync(enabled)
}

fun NativeTransport.clipboardSyncEnabled(): Boolean = preferences.getBoolean(CLIPBOARD_SYNC_KEY, false)

fun NativeTransport.setOverlayAssist(enabled: Boolean) {
    preferences.edit().putBoolean(OVERLAY_ASSIST_KEY, enabled).apply()
    configureClipboardLogMonitor()
}

fun NativeTransport.overlayAssistEnabled(): Boolean = preferences.getBoolean(OVERLAY_ASSIST_KEY, false)

fun NativeTransport.overlayPermissionGranted(): Boolean =
    android.provider.Settings.canDrawOverlays(context)

internal fun NativeTransport.shouldAssistBackgroundRead(clipWasNull: Boolean): Boolean =
    NativeTransport.shouldAssistBackgroundRead(
        clipboardSyncEnabled(),
        overlayAssistEnabled(),
        overlayPermissionGranted(),
        clipWasNull,
    )

internal fun NativeTransport.configureClipboardSync(enabled: Boolean) {
    configureClipboardLogMonitor()
    val manager = context.getSystemService(android.content.ClipboardManager::class.java)
    clipboardListener?.let(manager::removePrimaryClipChangedListener)
    clipboardListener = null
    if (!enabled) return
    val listener = android.content.ClipboardManager.OnPrimaryClipChangedListener {
        if (serverFingerprint == null) return@OnPrimaryClipChangedListener
        val clip = manager.primaryClip
        if (clip == null) {
            return@OnPrimaryClipChangedListener
        }
        if (isSensitiveClip(clip)) return@OnPrimaryClipChangedListener
        val item = clip.getItemAt(0) ?: return@OnPrimaryClipChangedListener
        val text = item.coerceToText(context)?.toString() ?: ""
        if (text.toByteArray(Charsets.UTF_8).size > 32 * 1024) return@OnPrimaryClipChangedListener
        val hash = clipboardHash(text)
        if (remoteClipboardHash == hash) {
            remoteClipboardHash = null
            return@OnPrimaryClipChangedListener
        }
        sendClipboardPayload(manager.primaryClip)
    }
    clipboardListener = listener
    manager.addPrimaryClipChangedListener(listener)
}

internal fun NativeTransport.configureClipboardLogMonitor() {
    clipboardLogProcess?.destroy()
    clipboardLogProcess = null
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) return
    if (!shouldAssistBackgroundRead(true)) return
    if (context.checkSelfPermission(android.Manifest.permission.READ_LOGS) !=
        android.content.pm.PackageManager.PERMISSION_GRANTED) return

    Thread({
        try {
            val filter = if (Build.VERSION.SDK_INT > 35) {
                "E ClipboardService"
            } else {
                "ClipboardService:E"
            }
            val process = Runtime.getRuntime().exec(arrayOf("logcat", "-T", "1", filter, "*:S"))
            clipboardLogProcess = process
            process.inputStream.bufferedReader().useLines { lines ->
                lines.filter { it.contains(context.packageName) }.forEach {
                    context.startActivity(Intent(context, ClipboardReadActivity::class.java).apply {
                        addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or
                            Intent.FLAG_ACTIVITY_CLEAR_TASK or
                            Intent.FLAG_ACTIVITY_NO_ANIMATION)
                    })
                }
            }
        } catch (_: Exception) {
            // The assist remains unavailable if log access is revoked or
            // Android rejects the background activity launch.
        }
    }, "handover-clipboard-monitor").apply { isDaemon = true }.start()
}

internal fun NativeTransport.clipboardHash(text: String): String = MessageDigest.getInstance("SHA-256")
    .digest(text.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it) }

fun NativeTransport.sendClipboardToLinux(): Boolean {
    if (serverFingerprint == null) return false
    val manager = context.getSystemService(android.content.ClipboardManager::class.java)
    return sendClipboardPayload(manager.primaryClip)
}

fun NativeTransport.sendAutomaticClipboardToLinux(): Boolean {
    if (serverFingerprint == null) return false
    val manager = context.getSystemService(android.content.ClipboardManager::class.java)
    val clip = manager.primaryClip ?: return false
    if (isSensitiveClip(clip)) return false
    return sendClipboardPayload(clip)
}

internal fun NativeTransport.isSensitiveClip(clip: android.content.ClipData): Boolean =
    Build.VERSION.SDK_INT >= Build.VERSION_CODES.N &&
        clip.description.extras?.getBoolean("android.content.extra.IS_SENSITIVE", false) == true

internal fun NativeTransport.sendClipboardPayload(clip: android.content.ClipData?): Boolean {
    val item = clip?.getItemAt(0) ?: return false
    val text = item.coerceToText(context)?.toString() ?: ""
    if (text.toByteArray(Charsets.UTF_8).size > 32 * 1024) return false
    val html = item.htmlText
    val uri = item.uri?.toString()
    val mime = clip.description?.getMimeType(0)
    if (uri != null && mime != null && !mime.startsWith("text/"))
        return sendClipboardFile(item.uri!!, mime)
    val richSize = text.toByteArray(Charsets.UTF_8).size +
        (html?.toByteArray(Charsets.UTF_8)?.size ?: 0) +
        (uri?.toByteArray(Charsets.UTF_8)?.size ?: 0)
    if (richSize > 48 * 1024 || (html?.toByteArray(Charsets.UTF_8)?.size ?: 0) > 32 * 1024 ||
        (uri?.toByteArray(Charsets.UTF_8)?.size ?: 0) > 32 * 1024) return false
    send(JSONObject().put("type", "clipboard_post").put("protocol", 1).put("text", text).apply {
        if (!html.isNullOrEmpty()) put("html", html)
        if (!uri.isNullOrEmpty()) put("uri", uri)
    })
    return true
}

internal fun NativeTransport.sendClipboardFile(uri: Uri, mime: String): Boolean {
    if (serverFingerprint == null || socket?.isClosed != false || output == null) return false
    val metadata = runCatching { fileMetadata(context.contentResolver, uri, null) }.getOrNull() ?: return false
    if (metadata.second > 10 * 1024 * 1024) return false
    return enqueue {
        try {
            context.contentResolver.openInputStream(uri)?.use { input ->
                val transferId = UUID.randomUUID().toString().replace("-", "")
                writeNow(JSONObject().put("type", "clipboard_file").put("protocol", 1)
                    .put("transfer_id", transferId)
                    .put("name", metadata.first).put("size", metadata.second).put("mime", mime))
                val buffer = ByteArray(STREAM_BUFFER_BYTES)
                var remaining = metadata.second
                while (remaining > 0) {
                    val count = input.read(buffer, 0, minOf(buffer.size.toLong(), remaining).toInt())
                    if (count <= 0) throw java.io.EOFException("clipboard changed while reading")
                    synchronized(outputLock) { output?.write(buffer, 0, count) ?: throw java.io.IOException("disconnected") }
                    remaining -= count
                }
                synchronized(outputLock) { output?.flush() ?: throw java.io.IOException("disconnected") }
            } ?: throw java.io.FileNotFoundException(uri.toString())
        } catch (_: Exception) {
            socket?.close()
        }
    }
}
