package org.handover.android

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
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
                context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
                    .putString(URI, uri.toString())
                    .putLong(VERSION_CODE, update.versionCode)
                    .putString(VERSION_NAME, update.versionName)
                    .apply()
            }
        } finally {
            temporary.delete()
        }
    }

    fun pending(context: Context): PendingUpdate? {
        val preferences = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val uri = preferences.getString(URI, null)?.let(Uri::parse) ?: return null
        return PendingUpdate(uri, preferences.getLong(VERSION_CODE, 0),
            preferences.getString(VERSION_NAME, "").orEmpty())
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
}
