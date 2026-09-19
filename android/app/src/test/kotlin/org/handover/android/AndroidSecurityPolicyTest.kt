package org.handover.android

import java.nio.file.Files
import java.nio.file.Path
import org.junit.Assert.assertTrue
import org.junit.Test

/** Host-side checks for security properties that are easy to regress in XML or wiring. */
class AndroidSecurityPolicyTest {
    private val manifest: String by lazy { Files.readString(Path.of("src/main/AndroidManifest.xml")) }
    private val transport: String by lazy {
        Files.readString(Path.of("src/main/kotlin/org/handover/android/NativeTransport.kt"))
    }
    private val updater: String by lazy {
        Files.readString(Path.of("src/main/kotlin/org/handover/android/AppUpdater.kt"))
    }

    @Test
    fun nonPublicRuntimeComponentsStayProtected() {
        assertComponent(".ClipboardReadActivity", "android:exported=\"false\"")
        assertComponent(".HandoverForegroundService", "android:exported=\"false\"")
        assertComponent(".UpdateInstalledReceiver", "android:exported=\"false\"")
        assertComponent(".TestNotificationReceiver", "android:permission=\"org.handover.android.permission.TEST_CONTROL\"")
        assertComponent(".HandoverDeviceAdminReceiver", "android:permission=\"android.permission.BIND_DEVICE_ADMIN\"")
    }

    @Test
    fun privilegedFeaturesKeepExplicitGates() {
        assertTrue(transport.contains("Settings.canDrawOverlays(context)"))
        assertTrue(transport.contains("context.checkSelfPermission(android.Manifest.permission.READ_LOGS)"))
        assertTrue(transport.contains("shouldAssistBackgroundRead(true)"))
        assertTrue(updater.contains("canRequestPackageInstalls()"))
    }

    private fun assertComponent(name: String, requiredAttribute: String) {
        val start = manifest.indexOf("android:name=\"$name\"")
        assertTrue("missing manifest component $name", start >= 0)
        val end = manifest.indexOf('>', start)
        assertTrue("missing end of manifest component $name", end > start)
        assertTrue(
            "component $name is missing $requiredAttribute",
            manifest.substring(start, end).contains(requiredAttribute),
        )
    }
}
