package org.handover.android

import android.Manifest
import android.content.ComponentName
import android.content.Context
import android.content.pm.PackageManager
import android.provider.Settings
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeFalse
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AndroidSecurityPolicyInstrumentedTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val packageManager: PackageManager = context.packageManager

    @Test
    fun mergedManifestKeepsRuntimeComponentsPrivateOrProtected() {
        val packageInfo = packageManager.getPackageInfo(
            context.packageName,
            PackageManager.GET_ACTIVITIES or PackageManager.GET_SERVICES or
                PackageManager.GET_RECEIVERS,
        )
        val activities = packageInfo.activities.orEmpty().associateBy { it.name }
        val services = packageInfo.services.orEmpty().associateBy { it.name }
        val receivers = packageInfo.receivers.orEmpty().associateBy { it.name }

        assertFalse(activities.getValue("${context.packageName}.ClipboardReadActivity").exported)
        assertFalse(services.getValue("${context.packageName}.HandoverForegroundService").exported)
        assertEquals(
            Manifest.permission.BIND_QUICK_SETTINGS_TILE,
            services.getValue("${context.packageName}.ClipboardTileService").permission,
        )
        assertEquals(
            Manifest.permission.BIND_NOTIFICATION_LISTENER_SERVICE,
            services.getValue("${context.packageName}.HandoverNotificationService").permission,
        )
        assertFalse(receivers.getValue("${context.packageName}.UpdateInstalledReceiver").exported)
        assertEquals(
            "${context.packageName}.permission.TEST_CONTROL",
            receivers.getValue("${context.packageName}.TestNotificationReceiver").permission,
        )
        assertEquals(
            Manifest.permission.BIND_DEVICE_ADMIN,
            receivers.getValue("${context.packageName}.HandoverDeviceAdminReceiver").permission,
        )
    }

    @Test
    fun privilegedPermissionsAreDeclaredAndDeniedStateIsObservable() {
        val requested = packageManager.getPackageInfo(
            context.packageName,
            PackageManager.GET_PERMISSIONS,
        ).requestedPermissions.orEmpty().toSet()
        assertTrue(requested.contains(Manifest.permission.SYSTEM_ALERT_WINDOW))
        assertTrue(requested.contains(Manifest.permission.READ_LOGS))
        assertTrue(requested.contains(Manifest.permission.REQUEST_INSTALL_PACKAGES))

        assumeFalse("test device already grants READ_LOGS", hasPermission(Manifest.permission.READ_LOGS))
        assertEquals(PackageManager.PERMISSION_DENIED, context.checkSelfPermission(Manifest.permission.READ_LOGS))

        assumeFalse("test device already grants overlay access", Settings.canDrawOverlays(context))
        assertFalse(Settings.canDrawOverlays(context))
    }

    @Test
    fun callActionStopsAtAndroidPermissionBoundary() {
        assumeFalse("test device already grants CALL_PHONE", hasPermission(Manifest.permission.CALL_PHONE))
        val result = CallController.place(context, "5551234")
        assertFalse(result.accepted)
        assertEquals("permission_denied", result.failure)
    }

    private fun hasPermission(permission: String): Boolean =
        context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED
}
