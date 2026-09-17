package org.handover.android

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Package replacement stops the foreground connection service. Remind the
 * user to reconnect without discarding or pretending to recreate pairing. */
class UpdateInstalledReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_MY_PACKAGE_REPLACED) return
        AppUpdater.markReconnectNeeded(context)
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(
            CHANNEL, "Handover updates", NotificationManager.IMPORTANCE_HIGH,
        ))
        val open = PendingIntent.getActivity(
            context, 0,
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        manager.notify(NOTIFICATION_ID, android.app.Notification.Builder(context, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_notify_sync_noanim)
            .setColor(0xffff9800.toInt())
            .setContentTitle("Reconnect Handover")
            .setContentText("Update installed. Tap to restore the Linux connection.")
            .setContentIntent(open)
            .setAutoCancel(true)
            .build())
    }

    companion object {
        private const val CHANNEL = "handover_updates"
        private const val NOTIFICATION_ID = 74
    }
}
