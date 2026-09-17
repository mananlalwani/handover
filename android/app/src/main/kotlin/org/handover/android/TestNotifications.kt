package org.handover.android

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.Context
import android.content.Intent

/** Harmless local notification exercising the native post/update/remove path.
 * It carries a genuine RemoteInput reply action so the desktop inline-reply
 * path has something real to target. */
object TestNotifications {
    const val CHANNEL = "handover_test"
    const val NOTIFICATION_ID = 1001

    fun post(context: Context, updated: Boolean, counter: Int) {
        if (android.os.Build.VERSION.SDK_INT >= 33 &&
            context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) !=
            android.content.pm.PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, "Handover test", NotificationManager.IMPORTANCE_DEFAULT),
        )
        val replyIntent = PendingIntent.getBroadcast(
            context, 0,
            Intent(context, TestNotificationReceiver::class.java)
                .setAction(TestNotificationReceiver.ACTION_TEST_REPLY),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE,
        )
        val replyAction = android.app.Notification.Action.Builder(
            android.R.drawable.ic_menu_send, "Reply",
            replyIntent,
        ).addRemoteInput(
            RemoteInput.Builder(TestNotificationReceiver.EXTRA_REPLY_KEY).setLabel("Reply").build(),
        ).build()
        val text = if (updated) "Updated test content #$counter" else "Hello from Handover #$counter"
        manager.notify(
            NOTIFICATION_ID,
            android.app.Notification.Builder(context, CHANNEL)
                .setContentTitle("Handover test")
                .setContentText(text)
                .setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
                .setAutoCancel(true)
                .addAction(replyAction)
                .build(),
        )
    }

    fun remove(context: Context) {
        context.getSystemService(NotificationManager::class.java).cancel(NOTIFICATION_ID)
    }
}
