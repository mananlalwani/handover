package org.handover.android

import android.content.Context
import android.os.Build
import android.os.Environment
import android.os.ParcelFileDescriptor
import android.os.storage.StorageManager
import android.system.Os
import android.system.OsConstants
import java.io.Closeable
import java.io.File
import java.io.FileNotFoundException
import java.io.InputStream
import java.nio.file.Files

internal fun hasAllPhoneFilesAccess(): Boolean = Build.VERSION.SDK_INT >= 30 && Environment.isExternalStorageManager()

/** System volume names stay local; only relative names enter the transport. */
internal fun phoneStorageRoots(context: Context): Map<String, File> {
    if (Build.VERSION.SDK_INT < 30) throw SecurityException("all-files access requires Android 11")
    return context.getSystemService(StorageManager::class.java).storageVolumes
        .filter { it.state == Environment.MEDIA_MOUNTED || it.state == Environment.MEDIA_MOUNTED_READ_ONLY }
        .mapNotNull { volume ->
            val root = volume.directory ?: return@mapNotNull null
            val name = if (volume.isPrimary) "Internal storage" else "Storage ${volume.uuid ?: return@mapNotNull null}"
            if (runCatching { safeTreePathParts(name).single() }.isFailure) return@mapNotNull null
            name to root.canonicalFile
        }.toMap()
}

internal data class PhoneFileEntry(val name: String, val directory: Boolean, val size: Long?)
internal class PhoneFileSource(val name: String, val size: Long?, val uri: android.net.Uri?, val input: InputStream) : Closeable {
    override fun close() = input.close()
}

/**
 * Walk through held directory descriptors. O_NOFOLLOW applies at every segment,
 * so renaming a directory or replacing it with a symlink cannot escape a volume.
 * Every descriptor is read-only, and non-regular files cannot be downloaded.
 */
internal class LocalPhoneStorage(private val roots: Map<String, File>) {
    private fun descriptor(path: String, directory: Boolean): ParcelFileDescriptor {
        val closeOnExec = if (Build.VERSION.SDK_INT >= 27) OsConstants.O_CLOEXEC else 0
        val flags = OsConstants.O_RDONLY or OsConstants.O_NOFOLLOW or closeOnExec or OsConstants.O_NONBLOCK
        val raw = Os.open(path, flags, 0)
        return try {
            if (directory) require(OsConstants.S_ISDIR(Os.fstat(raw).st_mode))
            ParcelFileDescriptor.dup(raw)
        } finally { Os.close(raw) }
    }

    private fun resolve(path: String): ParcelFileDescriptor {
        val parts = safeTreePathParts(path)
        require(parts.isNotEmpty())
        val root = roots[parts.first()] ?: throw FileNotFoundException()
        var current = descriptor(root.canonicalPath, true)
        try {
            for ((index, part) in parts.drop(1).withIndex()) {
                val next = descriptor("/proc/self/fd/${current.fd}/$part", index < parts.size - 2)
                current.close()
                current = next
            }
            return current
        } catch (error: Exception) { current.close(); throw error }
    }

    fun list(path: String, active: () -> Unit): List<PhoneFileEntry> {
        active()
        if (path == ".") return roots.keys.sorted().map { PhoneFileEntry(it, true, null) }
        return resolve(path).use { directory ->
            require(OsConstants.S_ISDIR(Os.fstat(directory.fileDescriptor).st_mode))
            val entries = mutableListOf<PhoneFileEntry>()
            Files.newDirectoryStream(File("/proc/self/fd/${directory.fd}").toPath()).use { children ->
                var scanned = 0
                for (child in children) {
                    active()
                    if (scanned++ >= 512) break
                    val name = child.fileName.toString()
                    if (runCatching { safeTreePathParts(name).single() }.isFailure) continue
                    val stat = try { Os.lstat(child.toString()) } catch (_: android.system.ErrnoException) { continue }
                    val isDirectory = OsConstants.S_ISDIR(stat.st_mode)
                    if (!isDirectory && !OsConstants.S_ISREG(stat.st_mode)) continue
                    entries += PhoneFileEntry(name, isDirectory, if (isDirectory) null else stat.st_size)
                }
            }
            entries.sortedWith(compareByDescending<PhoneFileEntry> { it.directory }.thenBy { it.name.lowercase() })
        }
    }

    fun open(path: String): PhoneFileSource {
        val handle = resolve(path)
        try {
            val stat = Os.fstat(handle.fileDescriptor)
            require(OsConstants.S_ISREG(stat.st_mode))
            return PhoneFileSource(safeTreePathParts(path).last(), stat.st_size, null, ParcelFileDescriptor.AutoCloseInputStream(handle))
        } catch (error: Exception) { handle.close(); throw error }
    }
}
