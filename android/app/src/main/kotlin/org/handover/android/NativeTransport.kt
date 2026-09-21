package org.handover.android

import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.content.ComponentName
import android.content.ContentResolver
import android.content.ContentValues
import android.net.Uri
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.media.RingtoneManager
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.Handler
import android.os.Looper
import android.os.Environment
import android.provider.MediaStore
import android.provider.ContactsContract
import android.util.Log
import android.util.Base64
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import java.io.ByteArrayOutputStream
import org.json.JSONObject
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.DataInputStream
import java.io.EOFException
import java.io.File
import java.io.FileOutputStream
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.Socket
import java.net.SocketTimeoutException
import java.nio.ByteBuffer
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.security.SecureRandom
import java.security.MessageDigest
import java.security.cert.X509Certificate
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.ConcurrentHashMap
import java.util.UUID
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import javax.net.ssl.X509TrustManager
import org.handover.android.DeviceIdentityStore.Companion.fingerprint

/** Native Handover transport. Frames are 4-byte big-endian length + UTF-8 JSON, max 64 KiB. */
class NativeTransport(internal val context: Context) {
    internal val callObserver = CallObserver(context) { frame ->
        if (serverFingerprint != null) send(frame)
    }
    internal val identity = DeviceIdentityStore(context)
    internal val nsd = context.getSystemService(NsdManager::class.java)
    internal val multicastLock = context.getSystemService(WifiManager::class.java)
        .createMulticastLock("handover-discovery").apply { setReferenceCounted(false) }
    internal val executor: ExecutorService = Executors.newSingleThreadExecutor()
    internal val writerExecutor = ThreadPoolExecutor(1, 1, 0, TimeUnit.MILLISECONDS,
        ArrayBlockingQueue(32)) { _, _ -> Thread { runCatching { socket?.close() } }.start() }
    internal val transferExpiry = Executors.newSingleThreadScheduledExecutor()
    internal val pendingTransfers = ConcurrentHashMap<String, PendingTransfer>()
    internal val preferences = context.getSharedPreferences("handover_native_peers", Context.MODE_PRIVATE)
    @Volatile internal var socket: SSLSocket? = null
    @Volatile internal var output: BufferedOutputStream? = null
    internal val outputLock = Any()
    @Volatile internal var serverId: String? = null
    @Volatile internal var serverFingerprint: String? = null
    @Volatile internal var pendingCode: String? = null
    @Volatile internal var approvalGranted = false
    @Volatile internal var ownNonce: String? = null
    @Volatile internal var serverCommit: String? = null
    @Volatile internal var discovery: NsdManager.DiscoveryListener? = null
    @Volatile internal var endpoint: Pair<InetAddress, Int>? = null
    @Volatile internal var manualEndpoint = false
    @Volatile internal var workerStarted = false
    @Volatile internal var remoteClipboardHash: String? = null
    @Volatile internal var connectionStatus = "offline"
    @Volatile internal var running = false
    internal var clipboardListener: android.content.ClipboardManager.OnPrimaryClipChangedListener? = null
    @Volatile internal var clipboardLogProcess: Process? = null
    internal var wakeLock: android.os.PowerManager.WakeLock? = null
    internal var connectionWakeLock: android.os.PowerManager.WakeLock? = null

    /** Reconnects to a stored endpoint after a restart when already paired. */
    fun connectToSavedEndpoint() {
        if (trustedPeerFingerprint(context) == null) return
        val manual = preferences.getString(MANUAL_ENDPOINT_KEY, null)
        val last = preferences.getString(LAST_ENDPOINT_KEY, null)
        val address = savedEndpointAddress(manual, last) ?: return
        applyEndpoint(address, persistManual = false, markManual = endpointIsManual(manual, address))
    }

    fun start() {
        transferExpiry.scheduleWithFixedDelay({ expireTransfers() }, TRANSFER_SWEEP_MS,
            TRANSFER_SWEEP_MS, TimeUnit.MILLISECONDS)
        serverFingerprint = preferences.getString(PIN_KEY, null)
        configureClipboardSync(preferences.getBoolean(CLIPBOARD_SYNC_KEY, false))
        multicastLock.acquire()
        discovery = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) { Log.i(TAG, "LAN discovery started") }
            override fun onDiscoveryStopped(serviceType: String) = Unit
            override fun onServiceFound(info: NsdServiceInfo) {
                if (info.serviceType.startsWith(SERVICE_TYPE)) {
                    Log.i(TAG, "Handover LAN service found")
                    nsd.resolveService(info, resolver)
                }
            }
            override fun onServiceLost(serviceInfo: NsdServiceInfo) {
                if (serviceInfo.serviceType.startsWith(SERVICE_TYPE)) {
                    Log.i(TAG, "LAN advertisement lost; keeping the current socket")
                }
            }
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                Log.w(TAG, "LAN discovery failed: $errorCode")
                runCatching { discovery?.let { nsd.stopServiceDiscovery(it) } }
            }
            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) = Unit
        }
        nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, discovery)
        running = true
        holdConnectionWakeLock()
        connectToSavedEndpoint()
        startWorker()
    }

    fun refreshCalls() = callObserver.refresh()

    fun stop() {
        running = false
        releaseWakeLock()
        releaseConnectionWakeLock()
        clipboardLogProcess?.destroy()
        clipboardLogProcess = null
        clipboardListener?.let {
            context.getSystemService(android.content.ClipboardManager::class.java)
                .removePrimaryClipChangedListener(it)
        }
        clipboardListener = null
        callObserver.stop()
        discovery?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        discovery = null
        if (multicastLock.isHeld) multicastLock.release()
        endpoint = null
        socket?.close()
        socket = null
        output = null
        executor.shutdownNow()
        writerExecutor.shutdownNow()
        failPendingTransfers(TRANSFER_DISCONNECTED)
        transferExpiry.shutdownNow()
    }

    fun connectionState(): String = connectionStatus

    /** Hold or release the phone screen at the desktop's request. True
     * means the wake lock changed as asked, never a battery guarantee. The
     * lock is best-effort, dim-level, and times out after ten minutes; the
     * desktop re-requests for longer sessions. */
    @Suppress("DEPRECATION")
    fun setPhoneAwake(inhibit: Boolean): Boolean = runCatching {
        if (inhibit) {
            if (wakeLock?.isHeld != true) {
                wakeLock = context.getSystemService(android.os.PowerManager::class.java)
                    .newWakeLock(android.os.PowerManager.SCREEN_DIM_WAKE_LOCK, "Handover:keep-awake")
                    .apply {
                        setReferenceCounted(false)
                        acquire(10 * 60 * 1000L)
                    }
            }
        } else {
            releaseWakeLock()
        }
        true
    }.getOrDefault(false)

    fun phoneAwakeHeld(): Boolean = wakeLock?.isHeld == true

    internal fun releaseWakeLock() {
        runCatching { if (wakeLock?.isHeld == true) wakeLock?.release() }
        wakeLock = null
    }

    /** Ask the paired desktop to hold (or release) its idle inhibitor. The
     * daemon reports the outcome to desktop clients; the phone switch only
     * records the requested state. */
    fun requestDesktopAwake(inhibit: Boolean) {
        preferences.edit().putBoolean(DESKTOP_AWAKE_KEY, inhibit).apply()
        if (serverFingerprint == null) return
        send(JSONObject().put("type", "screensaver_control").put("protocol", 1)
            .put("request_id", java.util.UUID.randomUUID().toString().replace("-", ""))
            .put("inhibit", inhibit))
    }

    fun desktopAwakeRequested(): Boolean = preferences.getBoolean(DESKTOP_AWAKE_KEY, false)




    fun connectTo(rawAddress: String): Boolean =
        applyEndpoint(rawAddress, persistManual = true, markManual = true)

    internal fun applyEndpoint(rawAddress: String, persistManual: Boolean, markManual: Boolean): Boolean {
        Log.i(TAG, "LAN endpoint requested")
        val address = rawAddress.trim()
        val separator = address.lastIndexOf(':')
        if (separator <= 0) { Log.w(TAG, "Endpoint format invalid"); return false }
        val port = address.substring(separator + 1).toIntOrNull()?.takeIf { it in 1..65535 }
            ?: return false.also { Log.w(TAG, "Endpoint port invalid") }
        val host = runCatching { InetAddress.getByName(address.substring(0, separator)) }.getOrNull()
            ?: return false.also { Log.w(TAG, "Endpoint address invalid") }
        Log.i(TAG, "Endpoint accepted")
        manualEndpoint = markManual
        endpoint = host to port
        if (persistManual) preferences.edit().putString(MANUAL_ENDPOINT_KEY, address).apply()
        writerExecutor.execute { runCatching { socket?.close() } }
        startWorker()
        return true
    }

    internal fun startWorker() {
        if (!workerStarted) {
            workerStarted = true
            executor.execute { connectionLoop() }
        }
    }

    fun approvePair(code: String) {
        if (code != pendingCode || serverId == null) return
        approvalGranted = true
        send(JSONObject().put("type", "pair_confirm").put("protocol", 1).put("code", code))
    }

    fun revoke() {
        send(JSONObject().put("type", "revoke").put("protocol", 1))
        preferences.edit().remove(PIN_KEY).apply()
        serverFingerprint = null
    }

    fun publishBattery(reading: BatteryReading) {
        if (serverFingerprint == null) return
        send(JSONObject().put("type", "battery").put("protocol", 1)
            .put("percentage", reading.percentage).put("charging", reading.charging))
    }

    fun publishConnectivity() {
        if (serverFingerprint == null) return
        val manager = context.getSystemService(ConnectivityManager::class.java)
        val network = manager.activeNetwork
        val capabilities = network?.let(manager::getNetworkCapabilities)
        val transport = when {
            capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true -> "wifi"
            capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) == true -> "ethernet"
            capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) == true -> "cellular"
            capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_BLUETOOTH) == true -> "bluetooth"
            capabilities != null -> "other"
            else -> "none"
        }
        send(JSONObject().put("type", "connectivity").put("protocol", 1)
            .put("transport", transport)
            .put("validated", capabilities?.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED) == true)
            .put("metered", capabilities?.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED) != true))
    }

    fun presentationControl(action: String, deltaX: Int = 0, deltaY: Int = 0): Boolean {
        if (serverFingerprint == null || action !in setOf(
                "previous", "next", "start", "stop", "fullscreen", "pointer_move", "pointer_click",
            )) return false
        send(JSONObject().put("type", "presentation_control").put("protocol", 1)
            .put("action", action).put("delta_x", deltaX).put("delta_y", deltaY))
        return true
    }

    fun volumeControl(action: String): Boolean {
        if (serverFingerprint == null || action !in setOf("up", "down", "toggle_mute")) return false
        send(JSONObject().put("type", "volume_control").put("protocol", 1).put("action", action))
        return true
    }

    fun remoteInput(action: String, deltaX: Int = 0, deltaY: Int = 0,
                    button: Int = 0, text: String? = null): Boolean {
        if (serverFingerprint == null || action !in setOf("move", "click", "scroll", "type")) return false
        if (kotlin.math.abs(deltaX) > 2000 || kotlin.math.abs(deltaY) > 2000 ||
            button !in 0..5 || (text?.toByteArray(Charsets.UTF_8)?.size ?: 0) > 512) return false
        send(JSONObject().put("type", "remote_input_control").put("protocol", 1)
            .put("action", action).put("delta_x", deltaX).put("delta_y", deltaY)
            .put("button", button).apply { if (text != null) put("text", text) })
        return true
    }








    /** Never send notification content before the peer is authenticated. */
    fun publishNotification(notification: WireNotification) {
        if (serverFingerprint == null) return
        if (!HandoverNotificationService.isValidKey(notification.key)) return
        send(HandoverNotificationService.postJson(notification))
    }

    fun retractNotification(key: String) {
        if (serverFingerprint == null) return
        if (!HandoverNotificationService.isValidKey(key)) return
        send(JSONObject().put("type", "notification_removed").put("protocol", 1).put("key", key))
    }

    fun syncNotifications(enabled: Boolean, notifications: List<WireNotification>) {
        if (serverFingerprint == null) return
        if (!enabled && notifications.isNotEmpty()) return
        send(HandoverNotificationService.syncJson(enabled, notifications.take(64)))
    }

    /** Never send media state before the peer is authenticated. */
    fun publishMedia(session: WireMediaSession) {
        if (serverFingerprint == null) return
        if (!MediaObserver.isValidPlayer(session.player)) return
        send(MediaObserver.postJson(session))
    }

    fun retractMedia(player: String) {
        if (serverFingerprint == null) return
        if (!MediaObserver.isValidPlayer(player)) return
        send(JSONObject().put("type", "media_removed").put("protocol", 1).put("player", player))
    }

    fun syncMedia(sessions: List<WireMediaSession>) {
        if (serverFingerprint == null) return
        send(MediaObserver.syncJson(sessions.take(16)))
    }






    internal val resolver = object : NsdManager.ResolveListener {
        override fun onServiceResolved(info: NsdServiceInfo) {
            Log.i(TAG, "Handover LAN service resolved")
            if (!manualEndpoint) {
                endpoint = info.host to info.port
                rememberLastEndpoint(info.host, info.port)
                startWorker()
            }
        }
        override fun onResolveFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
            Log.w(TAG, "LAN service resolution failed: $errorCode")
        }
    }

    internal fun connectionLoop() {
        broadcast(ACTION_CONNECTION_STATE, JSONObject().put("state", "reconnecting"))
        while (running) {
            val target = endpoint
            try {
                if (target == null) {
                    Thread.sleep(RECONNECT_DELAY_MS)
                    continue
                }
                pendingCode = null
                preferences.edit().remove(PENDING_CODE_KEY).apply()
                approvalGranted = false
                serverId = null
                ownNonce = generatePairingNonce()
                serverCommit = null
                socket?.close()
                val raw = Socket().apply { connect(InetSocketAddress(target.first, target.second), 5_000) }
                val ssl = sslContext().socketFactory.createSocket(raw, target.first.hostAddress, target.second, true) as SSLSocket
                ssl.enabledProtocols = arrayOf("TLSv1.3")
                ssl.soTimeout = SOCKET_READ_TIMEOUT_MS.toInt()
                ssl.startHandshake()
                socket = ssl
                rememberLastEndpoint(target.first, target.second)
                broadcast(ACTION_CONNECTION_STATE, JSONObject().put("state", "connected"))
                val input = BufferedInputStream(ssl.inputStream)
                output = BufferedOutputStream(ssl.outputStream)
                writeNow(hello())
                var missedPongs = 0
                while (!ssl.isClosed) {
                    val message = try { read(input) } catch (_: SocketTimeoutException) {
                        if (++missedPongs > 1) break
                        send(JSONObject().put("type", "ping").put("protocol", 1))
                        continue
                    } ?: break
                    missedPongs = 0
                    handle(message, input)
                }
            } catch (error: Exception) {
                Log.w(TAG, "LAN connection failed: ${error.javaClass.simpleName}")
                // Discovery remains active; retry the resolved endpoint after a bounded delay.
            } finally {
                broadcast(ACTION_CONNECTION_STATE, JSONObject().put("state", "offline"))
                failPendingTransfers(TRANSFER_DISCONNECTED)
                output = null
                socket?.close()
                socket = null
                pendingCode = null
                preferences.edit().remove(PENDING_CODE_KEY).apply()
                approvalGranted = false
                serverId = null
                ownNonce = null
                serverCommit = null
            }
            if (running) try { Thread.sleep(RECONNECT_DELAY_MS) } catch (_: InterruptedException) { return }
        }
    }

    internal fun rememberLastEndpoint(host: InetAddress, port: Int) {
        val text = host.hostAddress ?: return
        preferences.edit().putString(LAST_ENDPOINT_KEY, "$text:$port").apply()
    }

    internal fun holdConnectionWakeLock() {
        if (connectionWakeLock?.isHeld == true) return
        connectionWakeLock = context.getSystemService(android.os.PowerManager::class.java)
            .newWakeLock(android.os.PowerManager.PARTIAL_WAKE_LOCK, "Handover:connection")
            .apply {
                setReferenceCounted(false)
                acquire()
            }
    }

    internal fun releaseConnectionWakeLock() {
        runCatching { if (connectionWakeLock?.isHeld == true) connectionWakeLock?.release() }
        connectionWakeLock = null
    }

    internal fun handle(message: JSONObject, input: BufferedInputStream) {
        handleInbound(message, input)
    }


    internal fun sendDeviceCommandResult(
        requestId: String,
        action: String,
        accepted: Boolean,
        failure: String?,
    ) {
        send(deviceCommandResultJson(requestId, action, accepted, failure))
    }









    internal fun hello() = JSONObject().put("type", "hello").put("protocol", 1)
        .put("id", identity.deviceId).put("name", Build.MODEL ?: "Android device")
        .apply {
            serverFingerprint?.let { put("trusted_server_id", it) }
            ownNonce?.let { put("pair_commit", pairingCommitment(it)) }
        }

    internal fun sendBattery() {
        val intent = context.registerReceiver(null, android.content.IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val reading = intent?.let(BatteryObserver::reading) ?: return
        send(JSONObject().put("type", "battery").put("protocol", 1)
            .put("percentage", reading.percentage).put("charging", reading.charging))
    }

    internal fun writeNow(message: JSONObject) = synchronized(outputLock) {
        output?.let { stream -> runCatching { write(stream, message) }.onFailure { socket?.close() } }
    }









    internal data class PendingTransfer(
        val deadline: Long,
        val kind: String,
        val name: String,
        val uri: Uri?,
    ) {
        fun result(id: String, status: String) = JSONObject()
            .put("transfer_id", id).put("status", status).put("kind", kind).put("name", name)
            .apply { uri?.let { put("uri", it.toString()) } }
    }

    internal fun send(message: JSONObject) {
        enqueue { writeNow(message) }
    }

    internal fun enqueue(operation: () -> Unit): Boolean = try {
        if (writerExecutor.isShutdown) false else { writerExecutor.execute(operation); true }
    } catch (_: java.util.concurrent.RejectedExecutionException) { false }

    internal fun sslContext(): SSLContext {
        val keyManagers = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm()).apply {
            init(identity.keyStore(), null)
        }.keyManagers
        val trust = pinnedTrustManager(serverFingerprint)
        return SSLContext.getInstance("TLSv1.3").apply { init(keyManagers, arrayOf(trust), SecureRandom()) }
    }

    internal fun pinnedTrustManager(pin: String?): X509TrustManager =
        // This is deliberately custom: the first pairing handshake has no CA
        // trust anchor. It accepts only a non-empty, currently valid leaf for
        // the user-visible pairing ceremony; every subsequent handshake is
        // restricted to the stored certificate fingerprint. It never accepts
        // an empty chain and never disables TLS verification for a paired peer.
        PinnedIdentityTrustManager(pin)

    internal fun peerFingerprint(): String? = (socket?.session?.peerCertificates?.firstOrNull() as? X509Certificate)?.fingerprint()

    internal fun write(output: BufferedOutputStream, message: JSONObject) {
        val bytes = message.toString().toByteArray(Charsets.UTF_8)
        require(bytes.size <= MAX_FRAME)
        output.write(ByteBuffer.allocate(4).putInt(bytes.size).array())
        output.write(bytes)
        output.flush()
    }

    internal fun read(input: BufferedInputStream): JSONObject? {
        val data = DataInputStream(input)
        val header = ByteArray(4)
        val first = input.read()
        if (first < 0) return null
        header[0] = first.toByte()
        try { data.readFully(header, 1, 3) } catch (_: EOFException) { return null }
        val size = ByteBuffer.wrap(header).int
        if (size !in 1..MAX_FRAME) return null
        val payload = ByteArray(size)
        try { data.readFully(payload) } catch (_: EOFException) { return null }
        return runCatching { JSONObject(String(payload, Charsets.UTF_8)) }.getOrNull()
    }

    internal fun broadcast(action: String, payload: JSONObject) {
        if (action == ACTION_CONNECTION_STATE) {
            connectionStatus = payload.optString("state", "offline")
        }
        context.sendBroadcast(Intent(action).putExtra(EXTRA_JSON, payload.toString()).setPackage(context.packageName))
    }

    companion object {
        internal fun shouldAssistBackgroundRead(
            syncEnabled: Boolean,
            assistEnabled: Boolean,
            overlayGranted: Boolean,
            clipWasNull: Boolean,
        ): Boolean = syncEnabled && assistEnabled && overlayGranted && clipWasNull

        internal fun deviceCommandResultJson(
            requestId: String,
            action: String,
            accepted: Boolean,
            failure: String?,
        ): JSONObject = JSONObject(deviceCommandResultFields(requestId, action, accepted, failure))

        internal fun deviceCommandResultFields(
            requestId: String,
            action: String,
            accepted: Boolean,
            failure: String?,
        ): Map<String, Any> = buildMap {
            put("type", "device_command_result")
            put("protocol", 1)
            put("request_id", requestId)
            put("action", action)
            put("accepted", accepted)
            failure?.let { put("failure", it) }
        }

        const val ACTION_PAIR_REQUEST = "org.handover.android.PAIR_REQUEST"
        const val ACTION_PAIRED = "org.handover.android.PAIRED"
        const val ACTION_REVOKED = "org.handover.android.REVOKED"
        const val ACTION_CONNECTION_STATE = "org.handover.android.CONNECTION_STATE"
        const val EXTRA_JSON = "json"
        private const val SERVICE_TYPE = "_handover._tcp"
        internal const val TAG = "HandoverNative"
        internal const val PIN_KEY = "server_cert_sha256"
        internal const val PENDING_CODE_KEY = "pending_pair_code"
        private const val MANUAL_ENDPOINT_KEY = "manual_endpoint"
        private const val LAST_ENDPOINT_KEY = "last_endpoint"

        internal fun savedEndpointAddress(manual: String?, last: String?): String? =
            manual?.takeIf { it.isNotBlank() } ?: last?.takeIf { it.isNotBlank() }

        internal fun endpointIsManual(manual: String?, chosen: String?): Boolean =
            !manual.isNullOrBlank() && chosen == manual

        fun shouldAutoStartService(pairedFingerprint: String?): Boolean =
            !pairedFingerprint.isNullOrEmpty()
        private const val MAX_FRAME = 64 * 1024
        // Bounded snapshot contract: the whole contacts list must fit
        // one frame with envelope headroom. Per-field caps keep one
        // pathological record from eating the budget.
        internal const val CONTACTS_BUDGET_BYTES = 56 * 1024
        internal const val MAX_CONTACT_VALUES = 16
        internal const val MAX_CONTACT_FIELD_CHARS = 256
        internal const val CLIPBOARD_SYNC_KEY = "clipboard_sync_enabled"
        internal const val OVERLAY_ASSIST_KEY = "clipboard_overlay_assist"
        private const val DESKTOP_AWAKE_KEY = "desktop_awake_requested"
        internal const val MAX_FILE_BYTES = 100L * 1024 * 1024
        internal const val MAX_CLIPBOARD_FILE_BYTES = 10L * 1024 * 1024
        private const val MAX_URL_BYTES = 8 * 1024
        private const val MAX_NAME_BYTES = 255
        internal const val STREAM_BUFFER_BYTES = 32 * 1024
        internal const val SHARE_CHANNEL = "handover_received_shares"
        const val ACTION_REMOTE_RESULT = "org.handover.android.REMOTE_RESULT"
        private const val RECONNECT_DELAY_MS = 2_000L

        const val ACTION_SHARE_RECEIVED = "org.handover.android.SHARE_RECEIVED"
        const val ACTION_TRANSFER_RESULT = "org.handover.android.TRANSFER_RESULT"
        const val EXTRA_SHARE_URI = "uri"
        const val EXTRA_SHARE_TEXT = "text"
        internal const val MAX_PENDING_TRANSFERS = 32
        internal const val TRANSFER_TIMEOUT_MS = 120_000L
        internal const val INBOUND_TRANSFER_TIMEOUT_MS = 120_000L
        internal const val SOCKET_READ_TIMEOUT_MS = 30_000L
        private const val TRANSFER_SWEEP_MS = 5_000L
        internal const val TRANSFER_INVALID_RESOURCE = "invalid_resource"
        internal const val TRANSFER_SIZE_LIMIT = "size_limit"
        internal const val TRANSFER_STORAGE = "storage"
        internal const val TRANSFER_INTERRUPTED = "interrupted"
        internal const val TRANSFER_TIMED_OUT = "timed_out"
        internal const val TRANSFER_DISCONNECTED = "disconnected"
        private const val TRANSFER_REJECTED = "rejected"
        internal val TRANSFER_REASONS = setOf("invalid_resource", "size_limit", "storage",
            "interrupted", "rejected", "timed_out", "disconnected", "transport")

        internal fun transferDeadlineExpired(deadlineNanos: Long, nowNanos: Long = System.nanoTime()): Boolean =
            nowNanos >= deadlineNanos

        internal fun remainingTransferTimeoutMillis(
            deadlineNanos: Long,
            nowNanos: Long = System.nanoTime(),
        ): Long {
            val remainingNanos = (deadlineNanos - nowNanos).coerceAtLeast(1L)
            val millis = (remainingNanos + 999_999L) / 1_000_000L
            return millis.coerceIn(1L, SOCKET_READ_TIMEOUT_MS)
        }

        fun trustedPeerFingerprint(context: Context): String? =
            context.getSharedPreferences("handover_native_peers", Context.MODE_PRIVATE).getString(PIN_KEY, null)
        fun pendingPairingCode(context: Context): String? =
            context.getSharedPreferences("handover_native_peers", Context.MODE_PRIVATE).getString(PENDING_CODE_KEY, null)

        internal fun fileMetadata(resolver: ContentResolver, uri: Uri, requestedName: String?): Pair<String, Long>? {
            val name = requestedName ?: resolver.query(uri, arrayOf("_display_name"), null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) cursor.getString(0) else null
            } ?: uri.lastPathSegment?.substringAfterLast('/') ?: "shared-file"
            val safe = safeFileName(name) ?: return null
            val queriedSize = resolver.query(uri, arrayOf("_size"), null, null, null)?.use { cursor ->
                if (cursor.moveToFirst() && !cursor.isNull(0)) cursor.getLong(0) else -1L
            } ?: -1L
            val size = if (queriedSize >= 0) queriedSize else
                resolver.openAssetFileDescriptor(uri, "r")?.use { it.length } ?: -1L
            if (size !in 0..MAX_FILE_BYTES) return null
            return safe to size
        }

        internal fun safeFileName(raw: String): String? {
            val basename = raw.replace('\\', '/').substringAfterLast('/').trim()
            if (basename.isEmpty() || basename == "." || basename == ".." || basename.any { it.code < 0x20 || it == '\u0000' }) return null
            val bytes = basename.toByteArray(Charsets.UTF_8)
            if (bytes.size <= MAX_NAME_BYTES) return basename
            var end = MAX_NAME_BYTES
            while (end > 0 && (bytes[end].toInt() and 0xc0) == 0x80) end--
            return String(bytes, 0, end, Charsets.UTF_8).ifEmpty { null }
        }

        internal fun isValidShareUrl(value: String): Boolean {
            if (value.isEmpty() || value.toByteArray(Charsets.UTF_8).size > MAX_URL_BYTES ||
                value.trim() != value || value.any { it.isISOControl() || it.isWhitespace() }) return false
            val parsed = runCatching { java.net.URI(value) }.getOrNull() ?: return false
            return parsed.scheme?.lowercase() in setOf("http", "https")
        }

        internal fun isSafeBrowsePath(value: String): Boolean {
            if (value == ".") return true
            return value.length in 1..512 && !value.startsWith('/') && !value.contains('\u0000') &&
                value.split('/').all { part ->
                    part.isNotEmpty() && part != "." && part != ".." && !part.contains('\\')
                }
        }

        internal fun isSafeCommandName(value: String): Boolean =
            value.length in 1..64 && value.all { it in 'a'..'z' || it in '0'..'9' || it == '-' || it == '_' }

        internal fun isTransferId(value: String): Boolean =
            value.length == 32 && value.all { it in '0'..'9' || it in 'a'..'f' }

        internal fun uniqueDestination(root: File, name: String): File {
            var candidate = File(root, name)
            var suffix = 1
            while (candidate.exists()) {
                val dot = name.lastIndexOf('.')
                val stem = if (dot > 0) name.substring(0, dot) else name
                val extension = if (dot > 0) name.substring(dot) else ""
                candidate = File(root, "$stem ($suffix)$extension")
                suffix++
            }
            return candidate
        }

        fun pairingCode(ownFingerprint: String, ownNonce: String, peerFingerprint: String, peerNonce: String): String {
            // Each nonce stays bound to its fingerprint owner, so both sides
            // derive the same code without roles while a middlebox cannot
            // swap openings. Must match handover-native's comparison_code.
            val (lowFp, lowNonce, highFp, highNonce) = if (ownFingerprint <= peerFingerprint)
                Quad(ownFingerprint, ownNonce, peerFingerprint, peerNonce)
            else Quad(peerFingerprint, peerNonce, ownFingerprint, ownNonce)
            val data = "handover-pair-v2:$lowFp:$highFp:$lowNonce:$highNonce".toByteArray(Charsets.UTF_8)
            val digest = java.security.MessageDigest.getInstance("SHA-256").digest(data)
            val value = ByteBuffer.wrap(digest.copyOfRange(0, 4)).int.toLong() and 0xffffffffL
            return "%08d".format(value % 100000000L)
        }

        fun generatePairingNonce(): String {
            val bytes = ByteArray(16)
            SecureRandom().nextBytes(bytes)
            return bytes.joinToString("") { "%02x".format(it) }
        }

        fun pairingCommitment(nonceHex: String): String {
            val digest = java.security.MessageDigest.getInstance("SHA-256").digest(nonceHex.hexBytes())
            return digest.joinToString("") { "%02x".format(it) }
        }

        private data class Quad(val first: String, val second: String, val third: String, val fourth: String)

        internal fun String.hexBytes(): ByteArray {
            check(length % 2 == 0)
            return ByteArray(length / 2) { i -> substring(i * 2, i * 2 + 2).toInt(16).toByte() }
        }

        internal fun isSha256Hex(value: String) =
            value.length == 64 && runCatching { value.hexBytes() }.isSuccess

        internal fun isPairingNonce(value: String) =
            value.length == 32 && runCatching { value.hexBytes() }.isSuccess
    }
}
