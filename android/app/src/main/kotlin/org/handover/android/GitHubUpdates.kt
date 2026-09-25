package org.handover.android

import android.content.Context
import androidx.core.content.FileProvider
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.net.HttpURLConnection
import java.net.URL
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.security.MessageDigest
import org.json.JSONObject

internal data class GitHubApkAsset(val id: Long, val name: String, val size: Long, val sha256: String)
internal data class GitHubReleaseInfo(val version: String, val apk: GitHubApkAsset?)

private const val MAX_APK_BYTES = 100L * 1024 * 1024
private val VERSION = Regex("[0-9]+\\.[0-9]+\\.[0-9]+")
private val SHA256 = Regex("sha256:[0-9a-fA-F]{64}")

internal fun parseGitHubRelease(raw: String): GitHubReleaseInfo {
    val release = try { JSONObject(raw) } catch (_: Exception) { throw IOException("Invalid GitHub release") }
    if (release.optBoolean("draft") || release.optBoolean("prerelease")) {
        throw IOException("No published release")
    }
    val tag = release.optString("tag_name")
    val version = tag.removePrefix("v")
    if (!tag.startsWith("v") || !VERSION.matches(version)) throw IOException("Invalid release version")
    val expectedName = "handover-android-$version.apk"
    val assets = release.optJSONArray("assets") ?: throw IOException("Release assets unavailable")
    var apk: GitHubApkAsset? = null
    for (index in 0 until assets.length()) {
        val asset = assets.optJSONObject(index) ?: continue
        if (asset.optString("name") != expectedName || asset.optString("state") != "uploaded") continue
        val id = asset.optLong("id")
        val size = asset.optLong("size")
        val digest = asset.optString("digest")
        if (id > 0 && size in 1..MAX_APK_BYTES && SHA256.matches(digest)) {
            apk = GitHubApkAsset(id, expectedName, size, digest.removePrefix("sha256:").lowercase())
            break
        }
    }
    return GitHubReleaseInfo(version, apk)
}

internal fun isNewerRelease(release: String, installed: String): Boolean {
    if (!VERSION.matches(release) || !VERSION.matches(installed)) return false
    val next = release.split('.').map { it.toLongOrNull() ?: return false }
    val current = installed.split('.').map { it.toLongOrNull() ?: return false }
    for (index in 0..2) {
        if (next[index] != current[index]) return next[index] > current[index]
    }
    return false
}

internal fun copyVerifiedApk(input: InputStream, output: OutputStream, size: Long, sha256: String) {
    if (size !in 1..MAX_APK_BYTES || !Regex("[0-9a-fA-F]{64}").matches(sha256)) {
        throw IOException("Invalid APK metadata")
    }
    val digest = MessageDigest.getInstance("SHA-256")
    val buffer = ByteArray(64 * 1024)
    var copied = 0L
    while (true) {
        val count = input.read(buffer)
        if (count < 0) break
        if (count == 0) continue
        copied += count
        if (copied > size) throw IOException("APK size changed")
        output.write(buffer, 0, count)
        digest.update(buffer, 0, count)
    }
    if (copied != size) throw IOException("APK download incomplete")
    val actual = digest.digest().joinToString("") { "%02x".format(it) }
    if (!actual.equals(sha256, ignoreCase = true)) throw IOException("APK checksum mismatch")
}

internal sealed interface GitHubUpdateResult {
    data object Current : GitHubUpdateResult
    data class Ready(val update: PendingUpdate) : GitHubUpdateResult
    data class NoSignedApk(val version: String) : GitHubUpdateResult
    data object RejectedApk : GitHubUpdateResult
}

internal object GitHubUpdates {
    private const val RELEASE_URL = "https://api.github.com/repos/mananlalwani/handover/releases/latest"
    private const val ASSET_URL = "https://api.github.com/repos/mananlalwani/handover/releases/assets/"
    private const val MAX_RELEASE_BYTES = 1024 * 1024

    fun checkAndDownload(context: Context): GitHubUpdateResult {
        val connection = openGitHub(URL(RELEASE_URL), "application/vnd.github+json")
        val raw = try {
            connection.inputStream.use { input ->
                val output = ByteArrayOutputStream()
                val buffer = ByteArray(8192)
                while (true) {
                    val count = input.read(buffer)
                    if (count < 0) break
                    if (output.size() + count > MAX_RELEASE_BYTES) throw IOException("GitHub release too large")
                    output.write(buffer, 0, count)
                }
                output.toString(Charsets.UTF_8.name())
            }
        } finally { connection.disconnect() }
        val release = parseGitHubRelease(raw)
        @Suppress("DEPRECATION")
        val installed = context.packageManager.getPackageInfo(context.packageName, 0)
        if (!isNewerRelease(release.version, installed.versionName.orEmpty())) return GitHubUpdateResult.Current
        AppUpdater.pending(context)?.let { pending ->
            if (pending.versionName == release.version) return GitHubUpdateResult.Ready(pending)
        }
        val asset = release.apk ?: return GitHubUpdateResult.NoSignedApk(release.version)
        val directory = java.io.File(context.filesDir, "updates")
        if (!directory.isDirectory && !directory.mkdirs()) throw IOException("Update storage unavailable")
        val temporary = java.io.File.createTempFile(".handover-", ".part", directory)
        val destination = java.io.File(directory, asset.name)
        try {
            val download = openGitHub(URL(ASSET_URL + asset.id), "application/octet-stream")
            try {
                if (download.contentLengthLong > 0 && download.contentLengthLong != asset.size) {
                    throw IOException("APK size changed")
                }
                download.inputStream.use { input ->
                    temporary.outputStream().use { output ->
                        copyVerifiedApk(input, output, asset.size, asset.sha256)
                    }
                }
            } finally { download.disconnect() }
            Files.move(temporary.toPath(), destination.toPath(), StandardCopyOption.REPLACE_EXISTING)
            val uri = FileProvider.getUriForFile(context, "${context.packageName}.updates", destination)
            val verified = AppUpdater.inspectAndRemember(context, uri)
            if (verified == null) {
                destination.delete()
                return GitHubUpdateResult.RejectedApk
            }
            return GitHubUpdateResult.Ready(verified)
        } finally { temporary.delete() }
    }

    private fun openGitHub(initial: URL, accept: String): HttpURLConnection {
        var target = initial
        repeat(6) {
            val host = target.host.lowercase()
            if (target.protocol != "https" ||
                host != "api.github.com" && host != "github.com" &&
                !host.endsWith(".githubusercontent.com")
            ) throw IOException("Unexpected GitHub download host")
            val connection = (target.openConnection() as HttpURLConnection).apply {
                instanceFollowRedirects = false
                connectTimeout = 10_000
                readTimeout = 30_000
                setRequestProperty("Accept", accept)
                setRequestProperty("User-Agent", "Handover-Android")
            }
            val status = connection.responseCode
            if (status == HttpURLConnection.HTTP_OK) return connection
            if (status in 300..399) {
                val location = connection.getHeaderField("Location")
                connection.disconnect()
                if (location == null) throw IOException("GitHub redirect missing")
                target = URL(target, location)
            } else {
                connection.disconnect()
                throw IOException("GitHub returned HTTP $status")
            }
        }
        throw IOException("Too many GitHub redirects")
    }
}
