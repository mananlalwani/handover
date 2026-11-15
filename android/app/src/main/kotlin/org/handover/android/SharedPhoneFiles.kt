package org.handover.android

import android.content.ContentResolver
import android.net.Uri
import android.os.CancellationSignal
import android.provider.DocumentsContract
import org.json.JSONArray
import org.json.JSONObject
import java.io.Closeable
import java.io.File
import java.io.FileNotFoundException
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.net.SocketTimeoutException
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import org.handover.android.NativeTransport.Companion.MAX_FILE_BYTES
import org.handover.android.NativeTransport.Companion.STREAM_BUFFER_BYTES
import org.handover.android.NativeTransport.Companion.TRANSFER_INTERRUPTED
import org.handover.android.NativeTransport.Companion.isTransferId

internal fun NativeTransport.selectedShareTree(): Uri? =
    preferences.getString("shared_folder_tree", null)?.let(Uri::parse)

internal fun safeTreePathParts(path: String): List<String> {
    require(path.toByteArray(Charsets.UTF_8).size <= 512)
    require(!path.startsWith('/') && '\\' !in path && path.none(Char::isISOControl))
    if (path == ".") return emptyList()
    return path.split('/').also { parts ->
        require(parts.all { it.isNotEmpty() && it != "." && it != ".." && it.toByteArray(Charsets.UTF_8).size <= 255 })
    }
}

private val documentColumns = arrayOf(
    DocumentsContract.Document.COLUMN_DOCUMENT_ID,
    DocumentsContract.Document.COLUMN_DISPLAY_NAME,
    DocumentsContract.Document.COLUMN_MIME_TYPE,
    DocumentsContract.Document.COLUMN_SIZE,
    DocumentsContract.Document.COLUMN_FLAGS,
)
private data class PhoneDocument(val uri: Uri, val name: String, val directory: Boolean, val virtual: Boolean, val size: Long?)

private fun document(cursor: android.database.Cursor, root: Uri): PhoneDocument {
    fun index(name: String) = cursor.getColumnIndexOrThrow(name)
    val size = index(DocumentsContract.Document.COLUMN_SIZE)
    return PhoneDocument(
        DocumentsContract.buildDocumentUriUsingTree(root, cursor.getString(index(DocumentsContract.Document.COLUMN_DOCUMENT_ID))),
        cursor.getString(index(DocumentsContract.Document.COLUMN_DISPLAY_NAME)) ?: throw FileNotFoundException(),
        cursor.getString(index(DocumentsContract.Document.COLUMN_MIME_TYPE)) == DocumentsContract.Document.MIME_TYPE_DIR,
        cursor.getInt(index(DocumentsContract.Document.COLUMN_FLAGS)) and DocumentsContract.Document.FLAG_VIRTUAL_DOCUMENT != 0,
        if (cursor.isNull(size)) null else cursor.getLong(size).takeIf { it >= 0 },
    )
}

private fun resolveTreePath(resolver: ContentResolver, root: Uri, path: String, cancellation: CancellationSignal, active: () -> Unit): PhoneDocument {
    active()
    val rootUri = DocumentsContract.buildDocumentUriUsingTree(root, DocumentsContract.getTreeDocumentId(root))
    var current = resolver.query(rootUri, documentColumns, null, null, null, cancellation)?.use { cursor ->
        if (!cursor.moveToFirst()) throw FileNotFoundException()
        document(cursor, root)
    } ?: throw FileNotFoundException()
    for (part in safeTreePathParts(path)) {
        active()
        require(current.directory && !current.virtual)
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(root, DocumentsContract.getDocumentId(current.uri))
        var matched: PhoneDocument? = null
        resolver.query(children, documentColumns, null, null, null, cancellation)?.use { cursor ->
            var scanned = 0
            while (cursor.moveToNext()) {
                active()
                if (++scanned > 8192) throw IOException("directory exceeds scan bound")
                if (cursor.getString(cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DISPLAY_NAME)) == part) {
                    require(matched == null)
                    matched = document(cursor, root)
                }
            }
        } ?: throw FileNotFoundException()
        current = matched ?: throw FileNotFoundException()
        require(!current.virtual)
    }
    return current
}

/** The JSON reply always uses the captured session, even if reconnect races the writer. */
private fun NativeTransport.folderReply(socket: javax.net.ssl.SSLSocket, output: java.io.BufferedOutputStream, reply: JSONObject) {
    enqueue {
        synchronized(outputLock) {
            if (this.socket === socket && this.output === output) {
                runCatching { write(output, reply) }.onFailure { runCatching { socket.close() } }
            }
        }
    }
}

internal fun NativeTransport.handleDirectoryRequest(requestId: String, path: String) {
    val capturedSocket = socket ?: return
    val capturedOutput = output ?: return
    val root = selectedShareTree()
    val epoch = preferences.getLong("shared_folder_epoch", 0)
    val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(20)
    val cancellation = CancellationSignal()
    val abort = Closeable { cancellation.cancel() }
    fileRequestAborts.add(abort)
    val timeout = fileRequestTimeout.schedule({ abort.close() }, 20, TimeUnit.SECONDS)
    fun active() {
        if (System.nanoTime() >= deadline) throw SocketTimeoutException()
        if (socket !== capturedSocket || output !== capturedOutput) throw IOException()
        if (selectedShareTree() != root || preferences.getLong("shared_folder_epoch", 0) != epoch) throw SecurityException()
    }
    fun failed(reason: String) = JSONObject().put("type", "filesystem_failure").put("protocol", 1)
        .put("request_id", requestId).put("path", path).put("reason", reason)
    try {
        fileRequestWorkers.execute {
            val response = try {
                if (root == null) throw SecurityException()
                val target = resolveTreePath(context.contentResolver, root, path, cancellation, ::active)
                require(target.directory)
                val children = DocumentsContract.buildChildDocumentsUriUsingTree(root, DocumentsContract.getDocumentId(target.uri))
                val entries = JSONArray()
                var bytes = 0
                context.contentResolver.query(children, documentColumns, null, null, null, cancellation)?.use { cursor ->
                    var scanned = 0
                    while (scanned++ < 512 && cursor.moveToNext()) {
                        active()
                        val item = document(cursor, root)
                        if (item.virtual || runCatching { safeTreePathParts(item.name).single() }.isFailure) continue
                        val entry = JSONObject().put("name", item.name).put("directory", item.directory)
                        item.size?.let { if (!item.directory) entry.put("size", it) }
                        bytes += entry.toString().toByteArray(Charsets.UTF_8).size + 1
                        if (bytes > 48 * 1024) break
                        entries.put(entry)
                    }
                } ?: throw FileNotFoundException()
                active()
                JSONObject().put("type", "filesystem_entries").put("protocol", 1)
                    .put("request_id", requestId).put("path", path).put("entries", entries)
            } catch (_: SecurityException) { failed("permission_required") }
            catch (_: Exception) { failed("unavailable") }
            finally { timeout.cancel(false); fileRequestAborts.remove(abort) }
            // A folder change invalidates even an already-built successful reply.
            enqueue {
                synchronized(outputLock) {
                    if (socket === capturedSocket && output === capturedOutput) {
                        val safeReply = if (selectedShareTree() != root || preferences.getLong("shared_folder_epoch", 0) != epoch) failed("permission_required") else response
                        runCatching { write(capturedOutput, safeReply) }.onFailure { runCatching { capturedSocket.close() } }
                    }
                }
            }
        }
    } catch (_: java.util.concurrent.RejectedExecutionException) {
        timeout.cancel(false)
        fileRequestAborts.remove(abort)
        folderReply(capturedSocket, capturedOutput, failed("unavailable"))
    }
}

internal fun copyBounded(input: InputStream, output: OutputStream, limit: Long, checkActive: () -> Unit = {}): Long {
    val buffer = ByteArray(STREAM_BUFFER_BYTES)
    var total = 0L
    while (true) {
        checkActive()
        val count = input.read(buffer)
        checkActive()
        if (count < 0) return total
        if (count == 0) continue
        if (count.toLong() > limit - total) throw FileTooLarge()
        output.write(buffer, 0, count)
        total += count
    }
}

internal fun NativeTransport.handleFileRequest(message: JSONObject) {
    val id = message.optString("transfer_id")
    val path = message.optString("path")
    if (!isTransferId(id)) return
    val capturedSocket = socket ?: return
    val capturedOutput = output ?: return
    val root = selectedShareTree()
    val epoch = preferences.getLong("shared_folder_epoch", 0)
    val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(120)
    val cancellation = CancellationSignal()
    val source = AtomicReference<Closeable?>()
    val sending = AtomicBoolean(false)
    fun active() {
        if (System.nanoTime() >= deadline) throw SocketTimeoutException()
        if (socket !== capturedSocket || output !== capturedOutput) throw IOException()
        if (selectedShareTree() != root || preferences.getLong("shared_folder_epoch", 0) != epoch) throw SecurityException()
    }
    fun fail(reason: String) = folderReply(capturedSocket, capturedOutput,
        JSONObject().put("type", "file_request_failure").put("protocol", 1).put("transfer_id", id).put("reason", reason))
    if (root == null) { fail("permission_required"); return }
    if (runCatching { require(safeTreePathParts(path).isNotEmpty()) }.isFailure) { fail("invalid_resource"); return }
    if (!fileRequestSlots.tryAcquire()) { fail("unavailable"); return }
    val abort = Closeable {
        if (sending.get()) runCatching { capturedSocket.close() }
        cancellation.cancel()
        runCatching { source.getAndSet(null)?.close() }
    }
    fileRequestAborts.add(abort)
    val timeout = fileRequestTimeout.schedule({ abort.close() }, 120, TimeUnit.SECONDS)
    fun finish() { timeout.cancel(false); fileRequestAborts.remove(abort); fileRequestSlots.release() }
    try {
        fileRequestWorkers.execute {
            var temp: File? = null
            var handedToWriter = false
            try {
                val item = resolveTreePath(context.contentResolver, root, path, cancellation, ::active)
                require(!item.directory && !item.virtual && item.name == safeTreePathParts(path).last())
                if ((item.size ?: 0) > MAX_FILE_BYTES) throw FileTooLarge()
                active()
                temp = File.createTempFile("phone-download-", ".part", context.cacheDir)
                fileRequestTemps.add(temp)
                active()
                val descriptor = context.contentResolver.openAssetFileDescriptor(item.uri, "r", cancellation) ?: throw FileNotFoundException()
                val total = descriptor.use {
                    source.set(it)
                    it.createInputStream().use { input ->
                        temp.outputStream().use { out -> copyBounded(input, out, MAX_FILE_BYTES, ::active) }
                    }
                }
                source.set(null)
                if (item.size != null && total != item.size) throw IOException("size changed")
                active()
                val staged = temp
                handedToWriter = enqueue {
                    var registered = false
                    try {
                        synchronized(outputLock) {
                            active()
                            if (registerTransfer("file", item.name, item.uri, id) == null) throw IOException("transfer limit")
                            registered = true
                            announceAccepted(id)
                            sending.set(true)
                            write(capturedOutput, JSONObject().put("type", "share_file").put("protocol", 1)
                                .put("transfer_id", id).put("name", item.name).put("size", total))
                            staged.inputStream().use { input -> copyBounded(input, capturedOutput, total, ::active) }
                            active()
                            capturedOutput.flush()
                        }
                    } catch (error: Exception) {
                        if (registered) completeTransfer(id, "failed", TRANSFER_INTERRUPTED)
                        if (sending.get()) runCatching { capturedSocket.close() }
                        else fail(if (error is SecurityException) "permission_required" else "interrupted")
                    } finally { staged.delete(); fileRequestTemps.remove(staged); finish() }
                }
            } catch (error: Exception) {
                fail(when (error) {
                    is FileTooLarge -> "size_limit"
                    is SecurityException -> "permission_required"
                    is IllegalArgumentException -> "invalid_resource"
                    is SocketTimeoutException -> "interrupted"
                    else -> "unavailable"
                })
            } finally {
                if (!handedToWriter) { temp?.let { it.delete(); fileRequestTemps.remove(it) }; finish() }
            }
        }
    } catch (_: java.util.concurrent.RejectedExecutionException) { finish(); fail("unavailable") }
}

private class FileTooLarge : IOException()
