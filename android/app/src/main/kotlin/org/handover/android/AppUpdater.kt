package org.handover.android

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.provider.MediaStore
import java.io.File

data class PendingUpdate(val uri: Uri, val versionCode: Long, val versionName: String)

/** Verifies an APK using Android's package parser and the certificate already
 * trusted for this installed app. Installation is always delegated to the
 * system package installer and therefore remains user-confirmed. */
object AppUpdater {
    private const val PREFS = "handover_updates"
    private const val URI = "uri"
    private const val VERSION_CODE = "version_code"
    private const val VERSION_NAME = "version_name"
    private const val RECONNECT = "reconnect_after_update"

    private fun deleteManagedPackage(context: Context, rawUri: String?) {
        val uri = rawUri?.let(Uri::parse) ?: return
        // Update packages are inserted into MediaStore by Handover itself.
        // A failed delete is harmless and must not block update state cleanup.
        runCatching { context.contentResolver.delete(uri, null, null) }
    }

    @Suppress("DEPRECATION")
    private fun packageInfo(context: Context, path: String) =
        context.packageManager.getPackageArchiveInfo(
            path,
            if (Build.VERSION.SDK_INT >= 28) PackageManager.GET_SIGNING_CERTIFICATES
            else PackageManager.GET_SIGNATURES,
        )

    @Suppress("DEPRECATION")
    private fun certificates(info: android.content.pm.PackageInfo): Set<String> =
        if (Build.VERSION.SDK_INT >= 28) {
            val signing = info.signingInfo ?: return emptySet()
            val signatures = if (signing.hasMultipleSigners()) signing.apkContentsSigners
                else signing.signingCertificateHistory
            signatures.map { it.toCharsString() }.toSet()
        } else info.signatures.orEmpty().map { it.toCharsString() }.toSet()

    @Suppress("DEPRECATION")
    private fun versionCode(info: android.content.pm.PackageInfo): Long =
        if (Build.VERSION.SDK_INT >= 28) info.longVersionCode else info.versionCode.toLong()

    fun inspectAndRemember(context: Context, uri: Uri): PendingUpdate? {
        val temporary = File.createTempFile("handover-update-", ".apk", context.cacheDir)
        return try {
            context.contentResolver.openInputStream(uri)?.use { input ->
                temporary.outputStream().use(input::copyTo)
            } ?: return null
            val candidate = packageInfo(context, temporary.absolutePath) ?: return null
            val installed = context.packageManager.getPackageInfo(
                context.packageName,
                if (Build.VERSION.SDK_INT >= 28) PackageManager.GET_SIGNING_CERTIFICATES
                else PackageManager.GET_SIGNATURES,
            )
            if (candidate.packageName != context.packageName ||
                versionCode(candidate) <= versionCode(installed) ||
                certificates(candidate) != certificates(installed)
            ) return null
            PendingUpdate(uri, versionCode(candidate), candidate.versionName.orEmpty()).also { update ->
                val preferences = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                val previous = preferences.getString(URI, null)
                if (previous != null && previous != uri.toString()) {
                    deleteManagedPackage(context, previous)
                }
                preferences.edit()
                    .putString(URI, uri.toString())
                    .putLong(VERSION_CODE, update.versionCode)
                    .putString(VERSION_NAME, update.versionName)
                    // Package-replaced broadcasts are not reliable on every
                    // OEM. Preserve this marker across installation; while the
                    // APK remains pending it is suppressed by reconnectNeeded.
                    .putBoolean(RECONNECT, true)
                    .apply()
            }
        } finally {
            temporary.delete()
        }
    }

    /** Recover detection if the transfer notification was dismissed or the
     * app was closed when the APK arrived. Only scans Handover's directory. */
    fun scanDownloads(context: Context): PendingUpdate? {
        if (Build.VERSION.SDK_INT < 29) return null
        val projection = arrayOf(MediaStore.Downloads._ID)
        val updates = mutableListOf<PendingUpdate>()
        context.contentResolver.query(
            MediaStore.Downloads.EXTERNAL_CONTENT_URI, projection,
            "${MediaStore.Downloads.RELATIVE_PATH}=? AND ${MediaStore.Downloads.DISPLAY_NAME} LIKE ?",
            arrayOf("${android.os.Environment.DIRECTORY_DOWNLOADS}/Handover/", "%.apk"),
            "${MediaStore.Downloads.DATE_ADDED} DESC",
        )?.use { cursor ->
            val idIndex = cursor.getColumnIndexOrThrow(MediaStore.Downloads._ID)
            while (cursor.moveToNext() && updates.isEmpty()) {
                val uri = Uri.withAppendedPath(MediaStore.Downloads.EXTERNAL_CONTENT_URI,
                    cursor.getLong(idIndex).toString())
                inspectAndRemember(context, uri)?.let(updates::add)
            }
        }
        return updates.maxByOrNull { it.versionCode }
    }

    fun pending(context: Context): PendingUpdate? {
        val preferences = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val uri = preferences.getString(URI, null)?.let(Uri::parse) ?: return null
        val update = PendingUpdate(uri, preferences.getLong(VERSION_CODE, 0),
            preferences.getString(VERSION_NAME, "").orEmpty())
        @Suppress("DEPRECATION")
        val installed = context.packageManager.getPackageInfo(context.packageName, 0)
        val installedCode = if (Build.VERSION.SDK_INT >= 28) installed.longVersionCode
            else installed.versionCode.toLong()
        if (update.versionCode <= installedCode) {
            deleteManagedPackage(context, uri.toString())
            preferences.edit().remove(URI).remove(VERSION_CODE).remove(VERSION_NAME).apply()
            return null
        }
        return update
    }

    fun installIntent(context: Context, update: PendingUpdate): Intent =
        Intent(Intent.ACTION_VIEW).setDataAndType(update.uri, "application/vnd.android.package-archive")
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_GRANT_READ_URI_PERMISSION)

    fun requestInstall(context: Context, update: PendingUpdate) {
        if (Build.VERSION.SDK_INT >= 26 && !context.packageManager.canRequestPackageInstalls()) {
            context.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                Uri.parse("package:${context.packageName}")))
        } else {
            context.startActivity(installIntent(context, update))
        }
    }

    fun markReconnectNeeded(context: Context) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putBoolean(RECONNECT, true).apply()
    }

    fun reconnectNeeded(context: Context): Boolean =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getBoolean(RECONNECT, false) && pending(context) == null

    fun clearReconnectNeeded(context: Context) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putBoolean(RECONNECT, false).apply()
    }
}
