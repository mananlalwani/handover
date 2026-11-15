package org.handover.android

import android.content.Context
import android.os.Environment
import android.system.Os
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.BufferedOutputStream
import java.io.DataInputStream
import java.io.File
import java.io.PipedInputStream
import java.io.PipedOutputStream
import java.util.UUID
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket

@RunWith(AndroidJUnit4::class)
class PhoneStorageInstrumentedTest {
    private val context: Context = ApplicationProvider.getApplicationContext()

    @Test fun descriptorWalkPreservesBytesAndRejectsSymlinksTraversalAndDirectories() {
        val base = File(context.cacheDir, "storage-boundary-${UUID.randomUUID()}").apply { mkdirs() }
        try {
            val root = File(base, "shared").apply { mkdir() }
            val outside = File(base, "outside").apply { mkdir() }
            File(outside, "secret.txt").writeText("must not leave the shared root")
            File(root, "Code").mkdir()
            val bytes = byteArrayOf(0, 10, -1, 32, 65)
            File(root, "Code/original ✓.cc").writeBytes(bytes)
            Os.symlink(outside.path, File(root, "escape").path)
            Os.symlink(File(outside, "secret.txt").path, File(root, "secret-link").path)
            val storage = LocalPhoneStorage(mapOf("Internal storage" to root))
            assertEquals(listOf("Internal storage"), storage.list(".") {}.map { it.name })
            assertEquals(listOf("Code"), storage.list("Internal storage") {}.map { it.name })
            storage.open("Internal storage/Code/original ✓.cc").use {
                assertEquals("original ✓.cc", it.name)
                assertEquals(bytes.size.toLong(), it.size)
                assertArrayEquals(bytes, it.input.readBytes())
            }
            for (path in listOf("Internal storage/escape/secret.txt", "Internal storage/secret-link", "Internal storage/../outside/secret.txt", "Internal storage/Code", "/data/user/0/private")) {
                assertThrows(Exception::class.java) { storage.open(path).close() }
            }
        } finally { base.deleteRecursively() }
    }

    private fun withTransport(check: (NativeTransport, () -> JSONObject, DataInputStream) -> Unit) {
        val prefs = context.getSharedPreferences("handover_native_peers", Context.MODE_PRIVATE)
        val previous = prefs.all.filterKeys { it in setOf("shared_storage_all", "shared_folder_epoch", "shared_folder_tree") }
        val pipe = PipedInputStream(64 * 1024)
        val writer = PipedOutputStream(pipe)
        val input = DataInputStream(pipe)
        val transport = NativeTransport(context)
        val socket = SSLContext.getDefault().socketFactory.createSocket() as SSLSocket
        fun frame(): JSONObject {
            val length = input.readInt()
            require(length in 1..65536)
            val bytes = ByteArray(length)
            input.readFully(bytes)
            return JSONObject(String(bytes, Charsets.UTF_8))
        }
        try {
            prefs.edit().putBoolean("shared_storage_all", true).remove("shared_folder_tree")
                .putLong("shared_folder_epoch", prefs.getLong("shared_folder_epoch", 0) + 1).commit()
            transport.socket = socket
            transport.output = BufferedOutputStream(writer)
            transport.serverFingerprint = "fixture-peer"
            check(transport, ::frame, input)
        } finally {
            transport.stop()
            pipe.close(); writer.close()
            val edit = prefs.edit().remove("shared_storage_all").remove("shared_folder_epoch").remove("shared_folder_tree")
            previous.forEach { (key, value) -> when(value) {
                is Boolean -> edit.putBoolean(key, value)
                is Long -> edit.putLong(key, value)
                is String -> edit.putString(key, value)
            } }
            edit.commit()
        }
    }

    // Run once without the app-op and again after revocation from the test runner.
    // Android kills the app when all-files access changes, so do not mutate it in-process.
    @Test(timeout = 60000) fun deniedPermissionRejectsListingsAndDownloads() {
        org.junit.Assume.assumeFalse(hasAllPhoneFilesAccess())
        withTransport { transport, frame, _ ->
            transport.handleDirectoryRequest("00000000000000000000000000000001", ".")
            assertEquals("permission_required", frame().getString("reason"))
            transport.handleFileRequest(JSONObject().put("transfer_id", "00000000000000000000000000000002")
                .put("path", "Internal storage/Download/no-access.txt"))
            assertEquals("permission_required", frame().getString("reason"))
        }
    }

    @Test(timeout = 60000) fun grantedPermissionServesVolumesAndOriginalBytesAndStopSharingRevokesAccess() {
        org.junit.Assume.assumeTrue(hasAllPhoneFilesAccess())
        val folderName = "handover-fixture-${UUID.randomUUID()}"
        val fixture = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), folderName)
        try {
            withTransport { transport, frame, input ->
                transport.handleDirectoryRequest("00000000000000000000000000000001", ".")
                val listing = frame()
                assertEquals("filesystem_entries", listing.getString("type"))
                assertTrue((0 until listing.getJSONArray("entries").length()).any {
                    listing.getJSONArray("entries").getJSONObject(it).getString("name") == "Internal storage"
                })
                assertTrue(fixture.mkdirs())
                val bytes = "// source ✓\nint main() { return 0; }\n".toByteArray()
                File(fixture, "original.cc").writeBytes(bytes)
                val id = "00000000000000000000000000000003"
                val path = "Internal storage/Download/$folderName/original.cc"
                transport.handleFileRequest(JSONObject().put("transfer_id", id).put("path", path))
                val header = frame()
                assertEquals("share_file", header.getString("type"))
                assertEquals(id, header.getString("transfer_id"))
                assertEquals("original.cc", header.getString("name"))
                assertEquals(bytes.size.toLong(), header.getLong("size"))
                val downloaded = ByteArray(bytes.size)
                input.readFully(downloaded)
                assertArrayEquals(bytes, downloaded)
                context.getSharedPreferences("handover_native_peers", Context.MODE_PRIVATE).edit()
                    .putBoolean("shared_storage_all", false).commit()
                transport.handleDirectoryRequest("00000000000000000000000000000004", ".")
                assertEquals("permission_required", frame().getString("reason"))
                transport.handleFileRequest(JSONObject().put("transfer_id", "00000000000000000000000000000005").put("path", path))
                assertEquals("permission_required", frame().getString("reason"))
            }
        } finally { fixture.deleteRecursively() }
    }
}
