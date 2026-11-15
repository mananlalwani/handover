package org.handover.android

import android.content.Context
import android.os.Environment
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.DataInputStream
import java.io.File
import java.io.FileInputStream
import java.io.PipedInputStream
import java.io.PipedOutputStream
import java.io.RandomAccessFile
import java.security.MessageDigest
import java.util.UUID
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket

@RunWith(AndroidJUnit4::class)
class PhoneFileTransfersInstrumentedTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    @org.junit.Before fun permissionRequired() { assumeTrue(hasAllPhoneFilesAccess()) }

    private fun withTransfers(block: (PhoneFileTransfers, () -> JSONObject, NativeTransport, () -> Int) -> Unit) {
        assumeTrue("MANAGE_EXTERNAL_STORAGE must be granted by the test runner", hasAllPhoneFilesAccess())
        val prefs = context.getSharedPreferences("handover_native_peers", Context.MODE_PRIVATE)
        val previous = prefs.all.filterKeys { it in setOf("shared_storage_all", "shared_folder_epoch", "shared_folder_tree") }
        val incoming = PipedInputStream(128 * 1024)
        val writer = PipedOutputStream(incoming)
        val input = DataInputStream(BufferedInputStream(incoming))
        val transport = NativeTransport(context)
        val socket = SSLContext.getDefault().socketFactory.createSocket() as SSLSocket
        val module = PhoneFileTransfers(transport)
        fun frame(): JSONObject {
            val length = input.readInt()
            require(length in 1..65536)
            val bytes = ByteArray(length)
            input.readFully(bytes)
            val value=JSONObject(String(bytes, Charsets.UTF_8))
            if(value.optString("type")=="file_preparing_v2") {assertTrue(value.getLong("processed_bytes")>=0);return frame()}
            return value
        }
        try {
            prefs.edit().putBoolean("shared_storage_all", true).remove("shared_folder_tree")
                .putLong("shared_folder_epoch", prefs.getLong("shared_folder_epoch", 0) + 1).commit()
            transport.socket = socket
            transport.output = BufferedOutputStream(writer)
            transport.serverFingerprint = "instrumented-peer"
            block(module, ::frame, transport) { incoming.available() }
        } finally {
            module.close()
            transport.stop()
            runCatching { incoming.close() }; runCatching { writer.close() }
            val edit = prefs.edit().remove("shared_storage_all").remove("shared_folder_epoch").remove("shared_folder_tree")
            previous.forEach { (key, value) -> when (value) {
                is Boolean -> edit.putBoolean(key, value)
                is Long -> edit.putLong(key, value)
                is String -> edit.putString(key, value)
            } }
            edit.commit()
        }
    }

    private fun fixture(): File = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), "handover-v2-${UUID.randomUUID()}")
    private fun id(n: Int) = n.toString(16).padStart(32, '0')
    private fun hash(bytes: ByteArray) = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

    @Test(timeout = 60000) fun pagedSnapshotReturnsEveryEntryBeyondLegacyLimit() {
        val root = fixture()
        try {
            assertTrue(root.mkdirs())
            repeat(620) { File(root, "entry-%04d.txt".format(it)).writeText("fixture-$it") }
            withTransfers { module, frame, _, _ ->
                val path = "Internal storage/Download/${root.name}"
                val names = mutableSetOf<String>()
                var cursor: String? = null
                var request = 1
                do {
                    val message = JSONObject().put("type", "filesystem_list").put("protocol", 1)
                        .put("request_id", id(request++)).put("path", path).put("paged", true)
                    cursor?.let { message.put("cursor", it) }
                    module.handle(message)
                    val page = frame()
                    assertEquals("filesystem_entries", page.getString("type"))
                    val entries = page.getJSONArray("entries")
                    for (index in 0 until entries.length()) assertTrue(names.add(entries.getJSONObject(index).getString("name")))
                    cursor = page.optString("cursor_next").takeIf(String::isNotEmpty)
                } while (cursor != null)
                assertEquals(620, names.size)
                assertTrue(names.contains("entry-0619.txt"))
            }
        } finally { root.deleteRecursively() }
    }

    @Test(timeout = 60000) fun negotiatedVersionAndPositionedReadsReturnOriginalBytesAndCloseStopsReads() {
        val root = fixture()
        try {
            assertTrue(root.mkdirs())
            val original = ByteArray(90_000) { ((it * 31 + it / 13) and 0xff).toByte() }
            File(root, "source.bin").writeBytes(original)
            withTransfers { module, frame, _, available ->
                val transferId = id(20)
                val path = "Internal storage/Download/${root.name}/source.bin"
                module.handle(JSONObject().put("type", "file_open_v2").put("protocol", 1)
                    .put("transfer_id", transferId).put("path", path).put("expected_version", hash(original)))
                val info = frame()
                assertEquals("file_info_v2", info.getString("type"))
                assertEquals(hash(original), info.getString("version"))
                assertEquals(original.size.toLong(), info.getLong("size"))
                val offset = 27_321L
                val count = 4096
                module.handle(JSONObject().put("type", "file_read_v2").put("protocol", 1)
                    .put("transfer_id", transferId).put("offset", offset).put("size", count))
                val chunk = frame()
                assertEquals("file_chunk_v2", chunk.getString("type"))
                assertEquals(offset, chunk.getLong("offset"))
                assertArrayEquals(original.copyOfRange(offset.toInt(), offset.toInt() + count),
                    android.util.Base64.decode(chunk.getString("data"), android.util.Base64.NO_WRAP))

                module.handle(JSONObject().put("type", "file_close_v2").put("protocol", 1).put("transfer_id", transferId))
                module.handle(JSONObject().put("type", "file_read_v2").put("protocol", 1)
                    .put("transfer_id", transferId).put("offset", 0).put("size", 1))
                Thread.sleep(300)
                assertEquals("closed transfer must not emit another frame", 0, available())
            }
        } finally { root.deleteRecursively() }
    }

    @Test(timeout = 60000) fun changedExpectedVersionAndRevokedScopeRejectTransfer() {
        val root = fixture()
        try {
            assertTrue(root.mkdirs())
            val file = File(root, "mutable.bin")
            val old = "original fixture bytes".toByteArray()
            file.writeBytes(old)
            withTransfers { module, frame, transport, _ ->
                val path = "Internal storage/Download/${root.name}/mutable.bin"
                file.writeBytes("changed fixture bytes".toByteArray())
                module.handle(JSONObject().put("type", "file_open_v2").put("protocol", 1)
                    .put("transfer_id", id(30)).put("path", path).put("expected_version", hash(old)))
                val changed = frame()
                assertEquals("file_error_v2", changed.getString("type"))
                assertEquals("source_changed", changed.getString("reason"))

                val current = file.readBytes()
                val transferId = id(31)
                module.handle(JSONObject().put("type", "file_open_v2").put("protocol", 1)
                    .put("transfer_id", transferId).put("path", path).put("expected_version", hash(current)))
                assertEquals("file_info_v2", frame().getString("type"))
                val prefs = context.getSharedPreferences("handover_native_peers", Context.MODE_PRIVATE)
                prefs.edit().putLong("shared_folder_epoch", prefs.getLong("shared_folder_epoch", 0) + 1).commit()
                module.handle(JSONObject().put("type", "file_read_v2").put("protocol", 1)
                    .put("transfer_id", transferId).put("offset", 0).put("size", 1))
                val revoked = frame()
                assertEquals("file_error_v2", revoked.getString("type"))
                assertEquals("permission_required", revoked.getString("reason"))
                module.closeSession(transport.socket)
            }
        } finally { root.deleteRecursively() }
    }

    @Test(timeout = 60000) fun largeSparseCcSourceIsHashedAndServesPositionedBoundaryBytes() {
        val root = fixture()
        try {
            assertTrue(root.mkdirs())
            val name = "sparse source ✓.cc"
            val file = File(root, name)
            val size = 105L * 1024 * 1024
            RandomAccessFile(file, "rw").use { raf ->
                raf.setLength(size)
                raf.seek(0); raf.write(0x31)
                raf.seek(size - 1); raf.write(0x7a)
            }
            val expectedVersion = MessageDigest.getInstance("SHA-256").let { digest ->
                val buffer = ByteArray(64 * 1024)
                FileInputStream(file).use { input ->
                    while (true) {
                        val count = input.read(buffer)
                        if (count < 0) break
                        if (count > 0) digest.update(buffer, 0, count)
                    }
                }
                digest.digest().joinToString("") { "%02x".format(it) }
            }
            withTransfers { module, frame, _, _ ->
                val transferId = id(40)
                val path = "Internal storage/Download/${root.name}/$name"
                module.handle(JSONObject().put("type", "file_open_v2").put("protocol", 1)
                    .put("transfer_id", transferId).put("path", path).put("expected_version", expectedVersion))
                val info = frame()
                assertEquals("file_info_v2", info.getString("type"))
                assertEquals(name, info.getString("name"))
                assertEquals(size, info.getLong("size"))
                assertEquals(expectedVersion, info.getString("version"))
                for ((offset, expectedByte) in listOf(0L to 0x31, size - 1 to 0x7a)) {
                    module.handle(JSONObject().put("type", "file_read_v2").put("protocol", 1)
                        .put("transfer_id", transferId).put("offset", offset).put("size", 1))
                    val chunk = frame()
                    assertEquals("file_chunk_v2", chunk.getString("type"))
                    assertEquals(offset, chunk.getLong("offset"))
                    assertArrayEquals(byteArrayOf(expectedByte.toByte()),
                        android.util.Base64.decode(chunk.getString("data"), android.util.Base64.NO_WRAP))
                }
            }
        } finally { root.deleteRecursively() }
    }

    @Test(timeout = 60000) fun directoryCursorIsRejectedAfterSharingEpochChanges() {
        val root = fixture()
        try {
            assertTrue(root.mkdirs())
            repeat(160) { File(root, "cursor-entry-%03d.txt".format(it)).writeText("entry-$it") }
            withTransfers { module, frame, _, _ ->
                val path = "Internal storage/Download/${root.name}"
                module.handle(JSONObject().put("type", "filesystem_list").put("protocol", 1)
                    .put("request_id", id(50)).put("path", path).put("paged", true))
                val first = frame()
                assertEquals("filesystem_entries", first.getString("type"))
                val cursor = first.optString("cursor_next")
                assertTrue("directory needs a continuation cursor", cursor.isNotEmpty())
                val prefs = context.getSharedPreferences("handover_native_peers", Context.MODE_PRIVATE)
                prefs.edit().putLong("shared_folder_epoch", prefs.getLong("shared_folder_epoch", 0) + 1).commit()
                module.handle(JSONObject().put("type", "filesystem_list").put("protocol", 1)
                    .put("request_id", id(51)).put("path", path).put("paged", true).put("cursor", cursor))
                val failure = frame()
                assertEquals("filesystem_failure", failure.getString("type"))
                assertEquals("permission_required", failure.getString("reason"))
            }
        } finally { root.deleteRecursively() }
    }
}
