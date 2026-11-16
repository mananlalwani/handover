package org.handover.android

import android.view.View
import android.view.ViewGroup
import android.widget.TextView
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MainActivityNavigationInstrumentedTest {
    @org.junit.Before
    fun clearUpdatePromptForNavigationFixture() {
        AppUpdater.clearReconnectNeeded(androidx.test.core.app.ApplicationProvider.getApplicationContext())
    }

    private fun views(root: View): Sequence<View> = sequence {
        yield(root)
        if (root is ViewGroup) for (index in 0 until root.childCount) yieldAll(views(root.getChildAt(index)))
    }

    @Test
    fun everyHomeDestinationOpensAndReturnsWithoutLosingHomePosition() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { activity -> activity.pageScroll!!.scrollTo(0, 300) }
            instrumentation.waitForIdleSync()
            var homePosition = 0
            scenario.onActivity { activity ->
                homePosition = activity.pageScroll!!.scrollY
                assertTrue(homePosition > 0)
            }
            val destinations = listOf(
                "Notifications" to "Notifications",
                "Contacts" to "Contacts",
                "Activity" to "Activity",
                "Remote input" to "Remote input",
                "Presentation" to "Presentation remote",
                "Remote desktop" to "Remote desktop",
                "Laptop audio" to "Laptop audio",
                "System volume" to "System volume",
                "Connection" to "Connection",
                "Permissions" to "Permissions",
                "Updates" to "Updates",
                "Diagnostics" to "Diagnostics",
            )
            destinations.forEach { (label, title) ->
                scenario.onActivity { activity ->
                    val entry = views(activity.pageHost).filterIsInstance<TextView>()
                        .first { it.text.toString() == label }
                    (entry.parent as View).performClick()
                }
                instrumentation.waitForIdleSync()
                scenario.onActivity { activity ->
                    assertEquals("$label starts at the top", 0, activity.pageScroll!!.scrollY)
                    assertTrue("$label opens its page", views(activity.pageHost).filterIsInstance<TextView>()
                        .any { it.text.toString() == title && androidx.core.view.ViewCompat.isAccessibilityHeading(it) })
                    views(activity.pageHost).first { it.contentDescription == "Back to Handover" }.performClick()
                }
                instrumentation.waitForIdleSync()
                scenario.onActivity { activity ->
                    assertEquals("$label returns to the previous home position", homePosition, activity.pageScroll!!.scrollY)
                }
            }
        }
    }

    @Test
    @androidx.test.filters.SdkSuppress(minSdkVersion = 30)
    fun leavingNotificationSearchClosesTheKeyboard() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                val entry = views(activity.pageHost).filterIsInstance<TextView>()
                    .first { it.text.toString() == "Notifications" }
                (entry.parent as View).performClick()
            }
            instrumentation.waitForIdleSync()
            val focusDeadline = android.os.SystemClock.elapsedRealtime() + 5_000
            while (true) {
                var focused = false
                scenario.onActivity { focused = it.hasWindowFocus() }
                if (focused) break
                check(android.os.SystemClock.elapsedRealtime() < focusDeadline) { "Activity should have window focus" }
                Thread.sleep(25)
            }
            scenario.onActivity { activity ->
                val search = views(activity.pageHost).filterIsInstance<android.widget.EditText>().single()
                search.requestFocus()
                androidx.core.view.WindowCompat.getInsetsController(activity.window, search)
                    .show(androidx.core.view.WindowInsetsCompat.Type.ime())
            }
            fun awaitKeyboard(visible: Boolean) {
                val deadline = android.os.SystemClock.elapsedRealtime() + 5_000
                while (true) {
                    var actual = false
                    scenario.onActivity { activity ->
                        actual = activity.window.decorView.rootWindowInsets
                            ?.isVisible(android.view.WindowInsets.Type.ime()) == true
                    }
                    if (actual == visible) return
                    check(android.os.SystemClock.elapsedRealtime() < deadline) { "Keyboard visibility should become $visible" }
                    Thread.sleep(25)
                }
            }
            awaitKeyboard(true)
            instrumentation.waitForIdleSync()
            scenario.onActivity { activity ->
                val search = views(activity.pageHost).filterIsInstance<android.widget.EditText>().single()
                val bounds = android.graphics.Rect()
                assertTrue("Search stays visible with the keyboard open", search.getGlobalVisibleRect(bounds))
                val keyboard = activity.window.decorView.rootWindowInsets.getInsets(android.view.WindowInsets.Type.ime())
                assertTrue("Search stays above the keyboard", bounds.bottom <= activity.window.decorView.height - keyboard.bottom)
            }
            scenario.onActivity { activity ->
                views(activity.pageHost).first { it.contentDescription == "Back to Handover" }.performClick()
            }
            awaitKeyboard(false)
        }
    }

}
