package org.handover.android

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class DesktopShareInstrumentedTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val photo = Uri.parse("content://photos.example/item/1")

    private fun single(uri: Uri) = Intent(Intent.ACTION_SEND)
        .setType("image/jpeg").putExtra(Intent.EXTRA_STREAM, uri)

    private fun multiple(uris: List<Uri>) = Intent(Intent.ACTION_SEND_MULTIPLE)
        .setType("image/*").putParcelableArrayListExtra(Intent.EXTRA_STREAM, ArrayList(uris))

    @Test fun sharesheetHasOneNamedTargetForSingleAndMultipleFilesAndLinks() {
        for (intent in listOf(single(photo), multiple(listOf(photo)),
            Intent(Intent.ACTION_SEND).setType("text/plain"))) {
            intent.setPackage(context.packageName)
            val matches = context.packageManager.queryIntentActivities(intent, PackageManager.MATCH_DEFAULT_ONLY)
            assertEquals(1, matches.size)
            assertEquals("${context.packageName}.SendToPc", matches.single().activityInfo.name)
            assertEquals("Send to PC", matches.single().loadLabel(context.packageManager).toString())
        }
        val launcher = context.packageManager.getActivityInfo(
            ComponentName(context, MainActivity::class.java), 0)
        assertEquals("Handover", launcher.loadLabel(context.packageManager).toString())
    }

    @Test fun streamsTakePrecedenceOverAccompanyingTextAndKeepReadGrants() {
        val intent = single(photo).putExtra(Intent.EXTRA_TEXT, "https://example.com")
        val share = DesktopShare.fromIntent(intent) as DesktopShare.Files
        assertEquals(listOf(photo), share.uris)
        val service = share.serviceIntents(context).single()
        assertEquals(HandoverForegroundService.ACTION_SHARE_FILE, service.action)
        assertEquals(photo, service.getParcelableExtra<Uri>(HandoverForegroundService.EXTRA_URI))
        assertEquals(photo, service.clipData!!.getItemAt(0).uri)
        assertTrue(service.flags and Intent.FLAG_GRANT_READ_URI_PERMISSION != 0)
        assertEquals(ComponentName(context, HandoverForegroundService::class.java), service.component)
    }

    @Test fun multipleFilesRetainOrderAndRejectInvalidBatchBeforeSending() {
        val second = Uri.parse("content://files.example/document/2")
        val share = DesktopShare.fromIntent(multiple(listOf(photo, second))) as DesktopShare.Files
        assertEquals(listOf(photo, second), share.serviceIntents(context).map {
            it.getParcelableExtra<Uri>(HandoverForegroundService.EXTRA_URI)
        })
        assertNotNull(DesktopShare.fromIntent(multiple(List(32) { photo })))
        assertNull(DesktopShare.fromIntent(multiple(List(33) { photo })))
        assertNull(DesktopShare.fromIntent(multiple(emptyList())))
        assertNull(DesktopShare.fromIntent(multiple(listOf(photo, Uri.parse("file:///data/user/0/private")))))
        assertNull(DesktopShare.fromIntent(single(Uri.parse("content:///missing-authority"))))
        assertNull(DesktopShare.fromIntent(Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_STREAM, "bad")))
    }

    @Test fun urlsUseExistingProtocolAndUnsupportedTextIsRejected() {
        fun text(value: CharSequence) = Intent(Intent.ACTION_SEND)
            .setType("text/plain").putExtra(Intent.EXTRA_TEXT, value)
        val share = DesktopShare.fromIntent(text(StringBuilder(" https://example.com/path?q=1 ")))!!
        val service = share.serviceIntents(context).single()
        assertEquals(HandoverForegroundService.ACTION_SHARE_URL, service.action)
        assertEquals("https://example.com/path?q=1", service.getStringExtra(HandoverForegroundService.EXTRA_URL))
        assertNull(DesktopShare.fromIntent(text("some plain text")))
        assertNull(DesktopShare.fromIntent(text("javascript:alert(1)")))
        assertNull(DesktopShare.fromIntent(Intent(Intent.ACTION_VIEW).setData(photo)))
    }
}
