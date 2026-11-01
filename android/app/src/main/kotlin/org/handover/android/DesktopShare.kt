package org.handover.android

import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.net.Uri

/** A user-confirmed share to the one paired desktop, using existing transfers. */
internal sealed class DesktopShare {
    data class Files(val uris: List<Uri>) : DesktopShare()
    data class Url(val value: String) : DesktopShare()

    fun serviceIntents(context: Context): List<Intent> = when (this) {
        is Files -> uris.map { uri ->
            Intent(context, HandoverForegroundService::class.java)
                .setAction(HandoverForegroundService.ACTION_SHARE_FILE)
                .putExtra(HandoverForegroundService.EXTRA_URI, uri)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                .apply { clipData = ClipData.newRawUri("Shared file", uri) }
        }
        is Url -> listOf(Intent(context, HandoverForegroundService::class.java)
            .setAction(HandoverForegroundService.ACTION_SHARE_URL)
            .putExtra(HandoverForegroundService.EXTRA_URL, value))
    }

    companion object {
        // Match the transport's bounded set of pending transfers. A partly
        // invalid batch is rejected before any file is submitted.
        private const val MAX_FILES = 32

        @Suppress("DEPRECATION")
        fun fromIntent(intent: Intent): DesktopShare? = try {
            when (intent.action) {
                Intent.ACTION_SEND_MULTIPLE -> {
                    val uris = intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)
                    files(uris)
                }
                Intent.ACTION_SEND -> {
                    if (intent.hasExtra(Intent.EXTRA_STREAM)) {
                        files(intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM)?.let(::listOf))
                    } else {
                        val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()?.trim()
                        text?.takeIf { NativeTransport.isValidShareUrl(it) }?.let(::Url)
                    }
                }
                else -> null
            }
        } catch (_: Exception) {
            // Other apps control the extras. Never log their private payload.
            null
        }

        private fun files(uris: List<Uri>?): Files? {
            if (uris == null || uris.size !in 1..MAX_FILES) return null
            // Do not let an exported share target read app-private file:// paths.
            if (uris.any { it.scheme != "content" || it.authority.isNullOrEmpty() }) return null
            return Files(uris.toList())
        }
    }
}
