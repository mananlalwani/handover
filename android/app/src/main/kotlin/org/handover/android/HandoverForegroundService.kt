package org.handover.android

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.os.IBinder

/** Owns the native transport while the user has enabled Handover connectivity. */
class HandoverForegroundService : Service() {
    private lateinit var batteryObserver: BatteryObserver
    private lateinit var transport: NativeTransport

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
        startForeground(NOTIFICATION_ID, notification())
        transport = NativeTransport(this)
        transport.start()
        batteryObserver = BatteryObserver(this) { reading ->
            transport.publishBattery(reading)
        }
        batteryObserver.start()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = when (intent?.action) {
        ACTION_PAIR -> transport.approvePair(intent.getStringExtra(EXTRA_CODE).orEmpty()).let { START_STICKY }
        ACTION_REVOKE -> transport.revoke().let { START_STICKY }
        else -> START_STICKY
    }

    override fun onDestroy() {
        batteryObserver.stop()
        transport.stop()
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
        const val ACTION_PAIR = "org.handover.android.PAIR"
        const val ACTION_REVOKE = "org.handover.android.REVOKE"
        const val EXTRA_CODE = "code"
        private const val CHANNEL_ID = "handover_connection"
        private const val NOTIFICATION_ID = 1
    }
}
