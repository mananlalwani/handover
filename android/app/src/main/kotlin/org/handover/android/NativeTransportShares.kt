package org.handover.android

import android.annotation.SuppressLint
import android.content.ContentValues
import android.content.Intent
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.util.Log
import org.json.JSONObject
import java.io.BufferedInputStream
import java.io.EOFException
import java.io.File
import java.io.FileOutputStream
import java.net.SocketTimeoutException
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.UUID
import java.util.concurrent.TimeUnit
import org.handover.android.NativeTransport.Companion.ACTION_SHARE_RECEIVED
import org.handover.android.NativeTransport.Companion.ACTION_TRANSFER_RESULT
import org.handover.android.NativeTransport.Companion.INBOUND_TRANSFER_TIMEOUT_MS
import org.handover.android.NativeTransport.Companion.MAX_PENDING_TRANSFERS
import org.handover.android.NativeTransport.Companion.SHARE_CHANNEL
import org.handover.android.NativeTransport.Companion.SOCKET_READ_TIMEOUT_MS
import org.handover.android.NativeTransport.Companion.STREAM_BUFFER_BYTES
import org.handover.android.NativeTransport.Companion.TAG
import org.handover.android.NativeTransport.Companion.TRANSFER_INTERRUPTED
import org.handover.android.NativeTransport.Companion.TRANSFER_STORAGE
import org.handover.android.NativeTransport.Companion.TRANSFER_TIMEOUT_MS
import org.handover.android.NativeTransport.Companion.TRANSFER_TIMED_OUT
import org.handover.android.NativeTransport.Companion.fileMetadata
import org.handover.android.NativeTransport.Companion.isSafeBrowsePath
import org.handover.android.NativeTransport.Companion.isSafeCommandName
import org.handover.android.NativeTransport.Companion.isTransferId
import org.handover.android.NativeTransport.Companion.isValidShareUrl
import org.handover.android.NativeTransport.Companion.remainingTransferTimeoutMillis
import org.handover.android.NativeTransport.Companion.transferDeadlineExpired
import org.handover.android.NativeTransport.Companion.uniqueDestination

internal fun NativeTransport.listPhoneDirectory(requestId: String, path: String): JSONObject {
    val root = Environment.getExternalStorageDirectory().canonicalFile
    val target = File(root, path.trimStart('/')).canonicalFile
    require(target.path == root.path || target.path.startsWith(root.path + File.separator))
    require(target.isDirectory)
    val entries = org.json.JSONArray()
    target.listFiles()?.asSequence()?.sortedBy { it.name }?.take(512)?.forEach { file ->
        if (file.name.isEmpty() || file.name == "." || file.name == ".." ||
            file.name.any { it.isISOControl() } || file.name.contains('/')) return@forEach
        entries.put(JSONObject().put("name", file.name).put("directory", file.isDirectory)
            .apply { if (file.isFile) put("size", file.length()) })
    }
    return JSONObject().put("type", "filesystem_entries").put("protocol", 1)
        .put("request_id", requestId).put("path", path).put("entries", entries)
}
/** Sends a URL to the one explicitly paired desktop. */
fun NativeTransport.shareUrl(url: String): Boolean {
    if (serverFingerprint == null || socket?.isClosed != false || output == null || !isValidShareUrl(url)) return false
    val transferId = registerTransfer("url", url, Uri.parse(url)) ?: return false
    return if (enqueue {
        announceAccepted(transferId)
        writeNow(JSONObject().put("type", "share_url").put("protocol", 1)
            .put("transfer_id", transferId).put("url", url))
    }) true else {
        completeTransfer(transferId, "failed", TRANSFER_INTERRUPTED)
        false
    }
}

/**
 * Streams a content URI after its JSON header. The operation is serialized
 * with every other writer so raw bytes can never be mistaken for a frame.
 */
fun NativeTransport.shareFile(uri: Uri, requestedName: String? = null): Boolean {
    if (serverFingerprint == null || socket?.isClosed != false || output == null) return false
    val metadata = runCatching { fileMetadata(context.contentResolver, uri, requestedName) }.getOrNull() ?: return false
    val transferId = registerTransfer("file", metadata.first, uri) ?: return false
    return enqueue {
        announceAccepted(transferId)
        try {
            val resolver = context.contentResolver
            resolver.openInputStream(uri)?.use { input ->
                writeNow(JSONObject().put("type", "share_file").put("protocol", 1)
                    .put("transfer_id", transferId).put("name", metadata.first).put("size", metadata.second))
                val buffer = ByteArray(STREAM_BUFFER_BYTES)
                var remaining = metadata.second
                while (remaining > 0) {
                    val count = input.read(buffer, 0, minOf(buffer.size.toLong(), remaining).toInt())
                    if (count < 0) throw java.io.EOFException("file changed while sharing")
                    if (count == 0) continue
                    synchronized(outputLock) { output?.write(buffer, 0, count) ?: throw java.io.IOException("disconnected") }
                    remaining -= count
                }
                synchronized(outputLock) { output?.flush() ?: throw java.io.IOException("disconnected") }
            } ?: throw java.io.FileNotFoundException(uri.toString())
        } catch (error: Exception) {
            // A partial raw stream cannot be resynchronized as JSON. Drop
            // the authenticated session so the receiver deletes its temp.
            Log.w(TAG, "native file share failed: ${error.javaClass.simpleName}")
            completeTransfer(transferId, "failed", TRANSFER_INTERRUPTED)
            socket?.close()
        }
    }.also { accepted ->
        if (!accepted) completeTransfer(transferId, "failed", TRANSFER_INTERRUPTED)
    }
}

/** Request a directory listing from the paired Linux daemon. The daemon
 * must apply its own configured browse root before serving entries. */
fun NativeTransport.requestLinuxDirectory(path: String = "."): Boolean {
    if (serverFingerprint == null || !isSafeBrowsePath(path)) return false
    send(JSONObject().put("type", "filesystem_list").put("protocol", 1)
        .put("request_id", UUID.randomUUID().toString().replace("-", ""))
        .put("path", path))
    return true
}

/** Run a command from the Linux-side allowlist. The name is the only
 * command data accepted from the phone; argv never crosses this boundary. */
fun NativeTransport.runLinuxCommand(name: String): Boolean {
    if (serverFingerprint == null || !isSafeCommandName(name)) return false
    send(JSONObject().put("type", "custom_command_request").put("protocol", 1)
        .put("request_id", UUID.randomUUID().toString().replace("-", ""))
        .put("name", name))
    return true
}

fun NativeTransport.requestLinuxCommands(): Boolean {
    if (serverFingerprint == null) return false
    send(JSONObject().put("type", "custom_command_list_request").put("protocol", 1)
        .put("request_id", UUID.randomUUID().toString().replace("-", "")))
    return true
}
internal fun NativeTransport.receiveFile(input: BufferedInputStream, transferId: String, name: String, size: Long) {
    if (Build.VERSION.SDK_INT >= 29) {
        receivePublicDownload(input, transferId, name, size)
        return
    }
    val root = File(context.getExternalFilesDir(android.os.Environment.DIRECTORY_DOWNLOADS), "Handover")
    if (!root.exists() && !root.mkdirs()) {
        sendTransferFailureAndClose(transferId, TRANSFER_STORAGE); return
    }
    val destination = uniqueDestination(root, name)
    val temporary = File(root, ".${name}.${java.util.UUID.randomUUID()}.part")
    val deadline = inboundTransferDeadline()
    try {
        FileOutputStream(temporary).use { outputStream ->
            val buffer = ByteArray(STREAM_BUFFER_BYTES)
            var remaining = size
            while (remaining > 0) {
                checkInboundTransferDeadline(deadline)
                configureInboundReadTimeout(deadline)
                val count = input.read(buffer, 0, minOf(buffer.size.toLong(), remaining).toInt())
                if (count < 0) throw EOFException("interrupted file transfer")
                if (count == 0) continue
                outputStream.write(buffer, 0, count)
                remaining -= count
            }
            outputStream.fd.sync()
        }
        try {
            Files.move(temporary.toPath(), destination.toPath(), StandardCopyOption.ATOMIC_MOVE)
        } catch (_: java.nio.file.AtomicMoveNotSupportedException) {
            Files.move(temporary.toPath(), destination.toPath())
        }
        runCatching { notifyReceived("file", name, null, null) }
        broadcast(ACTION_SHARE_RECEIVED, JSONObject().put("kind", "file").put("name", name)
            .put("path", destination.absolutePath).put("source", serverId ?: serverFingerprint))
        TransferHistory.add(context, "file", name, null)
        sendTransferResult(transferId, "completed", null)
    } catch (error: Exception) {
        temporary.delete()
        Log.w(TAG, "native file receive failed: ${error.javaClass.simpleName}")
        sendTransferFailureAndClose(transferId, inboundFailureReason(error, deadline))
    } finally {
        restoreInboundReadTimeout()
    }
}

internal class InboundTransferTimeout : java.io.IOException("inbound transfer deadline exceeded")

internal fun NativeTransport.inboundTransferDeadline(): Long =
    System.nanoTime() + INBOUND_TRANSFER_TIMEOUT_MS * 1_000_000L

internal fun NativeTransport.checkInboundTransferDeadline(deadlineNanos: Long) {
    if (transferDeadlineExpired(deadlineNanos)) throw InboundTransferTimeout()
}

internal fun NativeTransport.configureInboundReadTimeout(deadlineNanos: Long) {
    socket?.soTimeout = remainingTransferTimeoutMillis(deadlineNanos).toInt()
}

internal fun NativeTransport.restoreInboundReadTimeout() {
    runCatching { socket?.soTimeout = SOCKET_READ_TIMEOUT_MS.toInt() }
}

internal fun NativeTransport.inboundFailureReason(error: Exception, deadlineNanos: Long): String = when {
    error is InboundTransferTimeout -> TRANSFER_TIMED_OUT
    error is SocketTimeoutException && transferDeadlineExpired(deadlineNanos) -> TRANSFER_TIMED_OUT
    error is EOFException -> TRANSFER_INTERRUPTED
    else -> TRANSFER_STORAGE
}

internal fun NativeTransport.receiveClipboardFile(
    input: BufferedInputStream,
    transferId: String,
    name: String,
    size: Long,
    mime: String,
) {
    if (Build.VERSION.SDK_INT < 29) {
        sendTransferFailureAndClose(transferId, TRANSFER_STORAGE)
        return
    }
    val values = android.content.ContentValues().apply {
        put(MediaStore.Downloads.DISPLAY_NAME, name)
        put(MediaStore.Downloads.MIME_TYPE, mime)
        put(MediaStore.Downloads.RELATIVE_PATH, "Download/Handover clipboard")
        put(MediaStore.Downloads.IS_PENDING, 1)
    }
    val uri = context.contentResolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
    if (uri == null) {
        sendTransferFailureAndClose(transferId, TRANSFER_STORAGE)
        return
    }
    val deadline = inboundTransferDeadline()
    try {
        context.contentResolver.openOutputStream(uri)?.use { output ->
            val buffer = ByteArray(STREAM_BUFFER_BYTES)
            var remaining = size
            while (remaining > 0) {
                checkInboundTransferDeadline(deadline)
                configureInboundReadTimeout(deadline)
                val count = input.read(buffer, 0, minOf(buffer.size.toLong(), remaining).toInt())
                if (count < 0) throw EOFException("interrupted clipboard transfer")
                if (count == 0) continue
                output.write(buffer, 0, count)
                remaining -= count
            }
            output.flush()
        } ?: throw java.io.IOException("clipboard output unavailable")
        context.contentResolver.update(uri, ContentValues().apply {
            put(MediaStore.Downloads.IS_PENDING, 0)
        }, null, null)
        context.getSystemService(android.content.ClipboardManager::class.java).setPrimaryClip(
            android.content.ClipData.newUri(context.contentResolver, name, uri),
        )
        sendTransferResult(transferId, "completed", null)
    } catch (error: Exception) {
        context.contentResolver.delete(uri, null, null)
        sendTransferFailureAndClose(
            transferId,
            inboundFailureReason(error, deadline),
        )
    } finally {
        restoreInboundReadTimeout()
    }
}

@SuppressLint("NewApi")
internal fun NativeTransport.receivePublicDownload(
    input: BufferedInputStream, transferId: String, name: String, size: Long,
) {
    val values = ContentValues().apply {
        put(MediaStore.Downloads.DISPLAY_NAME, name)
        put(MediaStore.Downloads.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/Handover")
        put(MediaStore.Downloads.IS_PENDING, 1)
    }
    val resolver = context.contentResolver
    val destination = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
    if (destination == null) {
        sendTransferFailureAndClose(transferId, TRANSFER_STORAGE)
        return
    }
    val deadline = inboundTransferDeadline()
    try {
        resolver.openOutputStream(destination, "w")!!.use { output ->
            val buffer = ByteArray(STREAM_BUFFER_BYTES)
            var remaining = size
            while (remaining > 0) {
                checkInboundTransferDeadline(deadline)
                configureInboundReadTimeout(deadline)
                val count = input.read(buffer, 0, minOf(buffer.size.toLong(), remaining).toInt())
                if (count < 0) throw EOFException("interrupted file transfer")
                if (count > 0) {
                    output.write(buffer, 0, count)
                    remaining -= count
                }
            }
        }
        resolver.update(destination, ContentValues().apply {
            put(MediaStore.Downloads.IS_PENDING, 0)
        }, null, null)
        notifyReceived("file", name, null, destination)
        broadcast(ACTION_SHARE_RECEIVED, JSONObject().put("kind", "file").put("name", name)
            .put("path", destination.toString()).put("source", serverId ?: serverFingerprint))
        TransferHistory.add(context, "file", name, destination)
        sendTransferResult(transferId, "completed", null)
    } catch (error: Exception) {
        resolver.delete(destination, null, null)
        Log.w(TAG, "native public download failed: ${error.javaClass.simpleName}")
        sendTransferFailureAndClose(
            transferId, inboundFailureReason(error, deadline),
        )
    } finally {
        restoreInboundReadTimeout()
    }
}

internal fun NativeTransport.notifyReceived(kind: String, value: String, url: String?, file: Uri? = null) {
    val manager = context.getSystemService(NotificationManager::class.java)
    manager.createNotificationChannel(NotificationChannel(SHARE_CHANNEL, "Received shares", NotificationManager.IMPORTANCE_DEFAULT))
    val source = (serverId ?: serverFingerprint ?: "unknown desktop").take(12)
    val builder = android.app.Notification.Builder(context, SHARE_CHANNEL)
        .setSmallIcon(android.R.drawable.stat_sys_download_done)
        .setAutoCancel(true)
    if (kind == "url") {
        builder.setContentTitle("URL received from $source")
            .setContentText("Tap to open the received URL")
        val parsed = url?.let { Uri.parse(it) }
        if (parsed != null && parsed.scheme?.lowercase() in setOf("http", "https")) {
            val intent = Intent(Intent.ACTION_VIEW, parsed)
            builder.setContentIntent(PendingIntent.getActivity(context, 0, intent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
        }
    } else {
        val update = file?.let { AppUpdater.inspectAndRemember(context, it) }
        builder.setContentTitle(if (update != null) "Handover update ready" else "File received from $source")
            .setContentText(if (update != null) "Version ${update.versionName.ifEmpty { update.versionCode.toString() }} · tap to install" else value)
        if (file != null) {
            val intent = if (update != null) AppUpdater.installIntent(context, update) else {
                val mime = context.contentResolver.getType(file)
                    ?: java.net.URLConnection.guessContentTypeFromName(value)
                    ?: "application/octet-stream"
                Intent(Intent.ACTION_VIEW).setDataAndType(file, mime)
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            builder.setContentIntent(PendingIntent.getActivity(
                context, file.hashCode(), intent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            ))
        }
    }
    manager.notify((System.currentTimeMillis() and 0x7fffffff).toInt(), builder.build())
}

internal fun NativeTransport.sendTransferResult(transferId: String, status: String, reason: String?) {
    if (!isTransferId(transferId)) return
    val result = JSONObject().put("type", "share_result").put("protocol", 1)
        .put("transfer_id", transferId).put("status", status)
    reason?.let { result.put("reason", it) }
    if (!enqueue { writeNow(result) }) socket?.close()
}

internal fun NativeTransport.sendTransferFailureAndClose(transferId: String, reason: String) {
    if (isTransferId(transferId)) {
        val result = JSONObject().put("type", "share_result").put("protocol", 1)
            .put("transfer_id", transferId).put("status", "failed").put("reason", reason)
        runCatching { writerExecutor.submit { writeNow(result) }.get(2, TimeUnit.SECONDS) }
    }
    socket?.close()
}

internal fun NativeTransport.registerTransfer(kind: String, name: String, uri: Uri?): String? {
    if (pendingTransfers.size >= MAX_PENDING_TRANSFERS) return null
    val id = UUID.randomUUID().toString().replace("-", "")
    pendingTransfers[id] = NativeTransport.PendingTransfer(
        android.os.SystemClock.elapsedRealtime() + TRANSFER_TIMEOUT_MS, kind, name, uri,
    )
    return id
}

internal fun NativeTransport.announceAccepted(id: String) {
    val transfer = pendingTransfers[id] ?: return
    publishTransferResult(transfer.result(id, "accepted"))
}

internal fun NativeTransport.completeTransfer(id: String, status: String, reason: String?) {
    val transfer = pendingTransfers.remove(id) ?: return
    val result = transfer.result(id, status)
    reason?.let { result.put("reason", it) }
    publishTransferResult(result)
}

internal fun NativeTransport.publishTransferResult(result: JSONObject) {
    TransferHistory.recordResult(
        context,
        result.optString("transfer_id"),
        result.optString("status"),
        result.optString("kind").takeIf(String::isNotEmpty),
        result.optString("name").takeIf(String::isNotEmpty),
        result.optString("uri").takeIf(String::isNotEmpty)?.let(Uri::parse),
    )
    broadcast(ACTION_TRANSFER_RESULT, result)
}

internal fun NativeTransport.expireTransfers() {
    val now = android.os.SystemClock.elapsedRealtime()
    val expired = pendingTransfers.entries.filter { it.value.deadline <= now }
    expired.forEach {
        completeTransfer(it.key, "failed", TRANSFER_TIMED_OUT)
    }
    if (expired.isNotEmpty()) socket?.close()
}

internal fun NativeTransport.failPendingTransfers(reason: String) {
    pendingTransfers.keys.toList().forEach { completeTransfer(it, "failed", reason) }
}
