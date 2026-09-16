package org.handover.android

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.os.IBinder

/** Lifecycle shell for the future native transport. It owns no protocol or pairing state yet. */
class HandoverForegroundService : Service() {
    private lateinit var batteryObserver: BatteryObserver

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
        startForeground(NOTIFICATION_ID, notification())
        batteryObserver = BatteryObserver(this) { reading ->
            // Transport integration will publish this normalized reading to handoverd.
            // Keep the callback intentionally side effect free until that boundary exists.
            @Suppress("UNUSED_VARIABLE") val current = reading
        }
        batteryObserver.start()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = START_STICKY

    override fun onDestroy() {
        batteryObserver.stop()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun createNotificationChannel() {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(CHANNEL_ID, "Handover connection", NotificationManager.IMPORTANCE_LOW))
    }

    private fun notification(): Notification = Notification.Builder(this, CHANNEL_ID)
        .setContentTitle("Handover is ready")
        .setContentText("Waiting for a trusted desktop connection")
        .setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
        .setOngoing(true)
        .build()

    companion object {
        private const val CHANNEL_ID = "handover_connection"
        private const val NOTIFICATION_ID = 1
    }
}
