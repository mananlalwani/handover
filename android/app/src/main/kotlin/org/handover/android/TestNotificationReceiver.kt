package org.handover.android

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Records replies to the harmless test notification so live runs can verify
 * the inline-reply path without involving a real messaging app. Also serves
 * scripted post/update/remove requests so the live acceptance run works over
 * `adb shell am broadcast` without screen taps. */
class TestNotificationReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            ACTION_TEST_REPLY -> {
                val results = android.app.RemoteInput.getResultsFromIntent(intent) ?: return
                val text = results.getCharSequence(EXTRA_REPLY_KEY)?.toString().orEmpty()
                context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                    .edit().putString(LAST_REPLY_KEY, text).apply()
                context.sendBroadcast(
                    Intent(ACTION_TEST_REPLY_RECEIVED).setPackage(context.packageName),
                )
            }
            ACTION_TEST_POST -> TestNotifications.post(context, false, intent.counter())
            ACTION_TEST_UPDATE -> TestNotifications.post(context, true, intent.counter())
            ACTION_TEST_REMOVE -> TestNotifications.remove(context)
        }
    }

    private fun Intent.counter(): Int =
        if (hasExtra(EXTRA_COUNTER)) getIntExtra(EXTRA_COUNTER, 1) else 1

    companion object {
        const val ACTION_TEST_REPLY = "org.handover.android.TEST_REPLY"
        const val ACTION_TEST_REPLY_RECEIVED = "org.handover.android.TEST_REPLY_RECEIVED"
        const val ACTION_TEST_POST = "org.handover.android.TEST_POST"
        const val ACTION_TEST_UPDATE = "org.handover.android.TEST_UPDATE"
        const val ACTION_TEST_REMOVE = "org.handover.android.TEST_REMOVE"
        const val EXTRA_REPLY_KEY = "handover_test_reply"
        const val EXTRA_COUNTER = "counter"
        const val PREFS = "handover_test"
        const val LAST_REPLY_KEY = "last_test_reply"

        fun lastReply(context: Context): String? =
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                .getString(LAST_REPLY_KEY, null)
    }
}
