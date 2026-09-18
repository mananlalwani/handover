package org.handover.android

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.net.Uri
import android.net.ConnectivityManager
import android.net.Network
import android.os.IBinder

/** Owns the native transport while the user has enabled Handover connectivity. */
class HandoverForegroundService : Service() {
    private lateinit var batteryObserver: BatteryObserver
    private lateinit var mediaObserver: MediaObserver
    private lateinit var transport: NativeTransport
    private lateinit var connectivityManager: ConnectivityManager
    private lateinit var connectivityCallback: ConnectivityManager.NetworkCallback

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
        startForeground(NOTIFICATION_ID, notification())
        transport = NativeTransport(applicationContext)
        connectivityManager = getSystemService(ConnectivityManager::class.java)
        connectivityCallback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) = transport.publishConnectivity()
            override fun onLost(network: Network) = transport.publishConnectivity()
            override fun onCapabilitiesChanged(network: Network, capabilities: android.net.NetworkCapabilities) {
                transport.publishConnectivity()
            }
        }
        connectivityManager.registerDefaultNetworkCallback(connectivityCallback)
        activeTransport = transport
        HandoverNotificationService.transport = transport
        MediaObserver.transport = transport
        transport.start()
        transport.connectToSavedEndpoint()
        batteryObserver = BatteryObserver(this) { reading ->
            transport.publishBattery(reading)
        }
        batteryObserver.start()
        mediaObserver = MediaObserver(this)
        mediaObserver.start()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = when (intent?.action) {
        ACTION_PAIR -> transport.approvePair(intent.getStringExtra(EXTRA_CODE).orEmpty()).let { START_STICKY }
        ACTION_REVOKE -> transport.revoke().let { START_STICKY }
        ACTION_CONNECT -> transport.connectTo(intent.getStringExtra(EXTRA_ADDRESS).orEmpty()).let { START_STICKY }
        ACTION_SHARE_URL -> transport.shareUrl(intent.getStringExtra(EXTRA_URL).orEmpty()).let { START_STICKY }
        ACTION_SHARE_FILE -> intent.getParcelableExtra<Uri>(EXTRA_URI)?.let { transport.shareFile(it) }.let { START_STICKY }
        else -> START_STICKY
    }

    override fun onDestroy() {
        batteryObserver.stop()
        mediaObserver.stop()
        connectivityManager.unregisterNetworkCallback(connectivityCallback)
        if (activeTransport === transport) activeTransport = null
        transport.stop()
        if (HandoverNotificationService.transport === transport) {
            HandoverNotificationService.transport = null
        }
        if (MediaObserver.transport === transport) {
            MediaObserver.transport = null
        }
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
        @Volatile private var activeTransport: NativeTransport? = null
        fun refreshCallsIfRunning() { activeTransport?.refreshCalls() }
        fun presentation(action: String, deltaX: Int = 0, deltaY: Int = 0) {
            activeTransport?.presentationControl(action, deltaX, deltaY)
        }

        const val ACTION_PAIR = "org.handover.android.PAIR"
        const val ACTION_REVOKE = "org.handover.android.REVOKE"
        const val ACTION_CONNECT = "org.handover.android.CONNECT"
        const val EXTRA_CODE = "code"
        const val EXTRA_ADDRESS = "address"
        const val ACTION_SHARE_URL = "org.handover.android.SHARE_URL"
        const val ACTION_SHARE_FILE = "org.handover.android.SHARE_FILE"
        const val EXTRA_URL = "url"
        const val EXTRA_URI = "uri"
        private const val CHANNEL_ID = "handover_connection"
        private const val NOTIFICATION_ID = 1
    }
}
