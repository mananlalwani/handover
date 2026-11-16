package org.handover.android

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.os.Process
import android.os.UserHandle
import android.service.notification.StatusBarNotification
import androidx.test.core.app.ApplicationProvider
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.ImageView
import android.widget.EditText
import android.widget.Switch
import android.widget.TextView
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class NotificationForwardingInstrumentedTest {
    private val context: Context = ApplicationProvider.getApplicationContext()

    private fun awaitCondition(description: String, condition: () -> Boolean) {
        val deadline = android.os.SystemClock.elapsedRealtime() + 10_000
        while (!condition()) {
            check(android.os.SystemClock.elapsedRealtime() < deadline) { description }
            Thread.sleep(25)
        }
    }

    private fun views(root: View): Sequence<View> = sequence {
        yield(root)
        if (root is ViewGroup) for (index in 0 until root.childCount) yieldAll(views(root.getChildAt(index)))
    }

    @Test
    fun phonePickerRemovesForwardedStateButKeepsThePhoneNotification() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        fun shell(command: String) {
            instrumentation.uiAutomation.executeShellCommand(command).use {
                java.io.FileInputStream(it.fileDescriptor).use { stream -> stream.readBytes() }
            }
        }
        val settings = NotificationForwarding(context)
        val previous = settings.isAllowed(context.packageName)
        val listenerEnabled = HandoverNotificationService.isEnabled(context)
        val manager = context.getSystemService(NotificationManager::class.java)
        val channel = "forwarding_fixture"
        val tag = "forwarding_fixture"
        shell("pm grant ${context.packageName} android.permission.POST_NOTIFICATIONS")
        shell("cmd notification allow_listener ${context.packageName}/.HandoverNotificationService")
        manager.createNotificationChannel(NotificationChannel(channel, "Forwarding test", NotificationManager.IMPORTANCE_LOW))
        try {
            settings.setAllowed(context.packageName, true)
            manager.notify(tag, 173, Notification.Builder(context, channel)
                .setSmallIcon(android.R.drawable.stat_notify_chat).setContentTitle("Forwarding test")
                .setContentText("Current phone notification").build())
            awaitCondition("Listener should expose the posted fixture") {
                HandoverNotificationService.snapshotFor(context).second.any { it.title == "Forwarding test" }
            }
            val key = manager.activeNotifications.first { it.tag == tag }.key
            ActivityScenario.launch(MainActivity::class.java).use { scenario ->
                fun openPicker() {
                    scenario.onActivity { activity ->
                        val title = views(activity.window.decorView).filterIsInstance<TextView>()
                            .first { it.text.toString() == "Notifications" }
                        (title.parent as View).performClick()
                        views(activity.window.decorView).filterIsInstance<EditText>()
                            .first { it.hint.toString() == "Search apps" }.setText(context.packageName)
                    }
                }
                fun pickerSwitch(activity: MainActivity): Switch? =
                    views(activity.window.decorView).filterIsInstance<Switch>()
                        .firstOrNull { it.tag == context.packageName }
                fun pickerLoaded(): Boolean {
                    var loaded = false
                    scenario.onActivity { loaded = pickerSwitch(it) != null }
                    return loaded
                }
                openPicker()
                awaitCondition("Picker should load the posting app") { pickerLoaded() }
                scenario.onActivity { pickerSwitch(it)!!.performClick() }
                assertFalse(NotificationForwarding(context).isAllowed(context.packageName))
                assertFalse(HandoverNotificationService.snapshotFor(context).second.any { it.key == key })
                assertTrue("The notification stays on the phone", manager.activeNotifications.any { it.key == key })
                assertFalse("A stale desktop dismissal must be rejected", HandoverNotificationService.dismiss(key))
                assertTrue(manager.activeNotifications.any { it.key == key })
                scenario.recreate()
                openPicker()
                awaitCondition("Picker should reload the saved choice") { pickerLoaded() }
                scenario.onActivity { assertFalse(pickerSwitch(it)!!.isChecked) }
                scenario.onActivity { pickerSwitch(it)!!.performClick() }
                assertTrue(HandoverNotificationService.snapshotFor(context).second.any { it.key == key })
                assertTrue(HandoverNotificationService.dismiss(key))
                awaitCondition("Explicit allowed dismissal should reach Android") {
                    manager.activeNotifications.none { it.key == key }
                }
            }
        } finally {
            settings.setAllowed(context.packageName, previous)
            manager.cancel(tag, 173)
            manager.deleteNotificationChannel(channel)
            if (!listenerEnabled) shell("cmd notification disallow_listener ${context.packageName}/.HandoverNotificationService")
        }
    }

    @Test
    fun bulkChoicesApplyToAllAppsWhileSearchingAndSurviveReload() {
        val preferences = context.getSharedPreferences("notification_forwarding", Context.MODE_PRIVATE)
        val previous = preferences.getStringSet("blocked_packages", emptySet()).orEmpty().toSet()
        val settings = NotificationForwarding(context)
        val apps = settings.apps()
        assertTrue("Fixture needs several apps to test filtered bulk changes", apps.size > 1)
        try {
            ActivityScenario.launch(MainActivity::class.java).use { scenario ->
                fun openPicker() {
                    scenario.onActivity { activity ->
                        val title = views(activity.window.decorView).filterIsInstance<TextView>()
                            .first { it.text.toString() == "Notifications" }
                        (title.parent as View).performClick()
                        views(activity.window.decorView).filterIsInstance<EditText>()
                            .first { it.hint.toString() == "Search apps" }.setText(context.packageName)
                    }
                    awaitCondition("Picker should load the app row") {
                        var loaded = false
                        scenario.onActivity { activity ->
                            loaded = views(activity.window.decorView).filterIsInstance<Switch>()
                                .any { it.tag == context.packageName }
                        }
                        loaded
                    }
                }
                fun click(label: String) {
                    scenario.onActivity { activity ->
                        views(activity.window.decorView).filterIsInstance<Button>()
                            .first { it.text.toString() == label }.performClick()
                    }
                }
                openPicker()
                scenario.onActivity { activity ->
                    val toggle = views(activity.window.decorView).filterIsInstance<Switch>()
                        .first { it.tag == context.packageName }
                    val row = toggle.parent as View
                    assertNotNull(views(row).filterIsInstance<ImageView>().single().drawable)
                    val label = views(row).filterIsInstance<TextView>().first { it.text.toString() == "Handover" }
                    val packageLabel = views(row).filterIsInstance<TextView>()
                        .first { it.text.toString() == context.packageName }
                    assertTrue(label.textSize > packageLabel.textSize)
                    assertTrue(label.typeface.isBold)
                }
                click("Block all")
                assertTrue("Bulk block includes apps outside the search", apps.all { !settings.isAllowed(it.packageName) })
                assertTrue("Future apps keep the documented default", settings.isAllowed("org.handover.uninstalled.fixture"))
                scenario.recreate()
                openPicker()
                scenario.onActivity { activity ->
                    assertFalse(views(activity.window.decorView).filterIsInstance<Switch>()
                        .first { it.tag == context.packageName }.isChecked)
                }
                click("Allow all")
                assertTrue(apps.all { NotificationForwarding(context).isAllowed(it.packageName) })
                scenario.onActivity { activity ->
                    val toggle = views(activity.window.decorView).filterIsInstance<Switch>()
                        .first { it.tag == context.packageName }
                    (toggle.parent as View).performClick()
                    assertFalse("The full row toggles forwarding", toggle.isChecked)
                }
                assertFalse(settings.isAllowed(context.packageName))
            }
        } finally {
            preferences.edit().putStringSet("blocked_packages", previous).commit()
            HandoverNotificationService.requestSync(context)
        }
    }

    @Test
    fun appChoiceFiltersBothNewAndUpdatedNotificationsAndSurvivesReload() {
        val settings = NotificationForwarding(context)
        val packageName = context.packageName
        val previous = settings.isAllowed(packageName)
        val notification = Notification.Builder(context, "fixture")
            .setSmallIcon(android.R.drawable.stat_notify_chat)
            .setContentTitle("Battery monitor")
            .setContentText("Battery at 81%")
            .build()
        val posted = StatusBarNotification(packageName, packageName, 42, "filter-fixture",
            Process.myUid(), Process.myPid(), 0, notification, UserHandle.getUserHandleForUid(Process.myUid()), 1L)
        try {
            settings.setAllowed(packageName, true)
            assertNotNull(HandoverNotificationService.snapshot(context, posted))
            settings.setAllowed(packageName, false)
            assertFalse(NotificationForwarding(context).isAllowed(packageName))
            assertNull("Opted-out app must not forward its current content",
                HandoverNotificationService.snapshot(context, posted))
            notification.extras.putCharSequence(Notification.EXTRA_TEXT, "Battery at 82%")
            assertNull("An update from the same opted-out app must stay blocked",
                HandoverNotificationService.snapshot(context, posted))
            assertTrue("Unrelated apps keep forwarding by default", settings.isAllowed("org.example.other"))
            settings.setAllowed(packageName, true)
            assertEquals("Battery at 82%", HandoverNotificationService.snapshot(context, posted)?.body)
        } finally {
            settings.setAllowed(packageName, previous)
        }
    }
}
