package org.handover.android

import android.content.Context
import android.content.Intent
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.util.Log
import org.json.JSONObject
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.DataInputStream
import java.io.EOFException
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.Socket
import java.net.SocketTimeoutException
import java.nio.ByteBuffer
import java.security.SecureRandom
import java.security.cert.X509Certificate
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import javax.net.ssl.X509TrustManager
import org.handover.android.DeviceIdentityStore.Companion.fingerprint

/** Native Handover transport. Frames are 4-byte big-endian length + UTF-8 JSON, max 64 KiB. */
class NativeTransport(private val context: Context) {
    private val identity = DeviceIdentityStore(context)
    private val nsd = context.getSystemService(NsdManager::class.java)
    private val multicastLock = context.getSystemService(WifiManager::class.java)
        .createMulticastLock("handover-discovery").apply { setReferenceCounted(false) }
    private val executor: ExecutorService = Executors.newSingleThreadExecutor()
    private val writerExecutor = ThreadPoolExecutor(1, 1, 0, TimeUnit.MILLISECONDS,
        ArrayBlockingQueue(32)) { _, _ -> Thread { runCatching { socket?.close() } }.start() }
    private val preferences = context.getSharedPreferences("handover_native_peers", Context.MODE_PRIVATE)
    @Volatile private var socket: SSLSocket? = null
    @Volatile private var output: BufferedOutputStream? = null
    private val outputLock = Any()
    @Volatile private var serverId: String? = null
    @Volatile private var serverFingerprint: String? = null
    @Volatile private var pendingCode: String? = null
    @Volatile private var approvalGranted = false
    @Volatile private var ownNonce: String? = null
    @Volatile private var serverCommit: String? = null
    @Volatile private var discovery: NsdManager.DiscoveryListener? = null
    @Volatile private var endpoint: Pair<InetAddress, Int>? = null
    @Volatile private var manualEndpoint = false
    @Volatile private var workerStarted = false

    /** Reconnects to the stored manual endpoint after a restart when already paired. */
    fun connectToSavedEndpoint() {
        if (trustedPeerFingerprint(context) == null) return
        preferences.getString(MANUAL_ENDPOINT_KEY, null)?.let(::connectTo)
    }

    fun start() {
        serverFingerprint = preferences.getString(PIN_KEY, null)
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
                if (!manualEndpoint && serviceInfo.serviceType.startsWith(SERVICE_TYPE)) {
                    endpoint = null
                    socket?.close()
                }
            }
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                Log.w(TAG, "LAN discovery failed: $errorCode")
                runCatching { discovery?.let { nsd.stopServiceDiscovery(it) } }
            }
            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) = Unit
        }
        nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, discovery)
        preferences.getString(MANUAL_ENDPOINT_KEY, null)?.let(::connectTo)
    }

    fun stop() {
        discovery?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        discovery = null
        if (multicastLock.isHeld) multicastLock.release()
        endpoint = null
        socket?.close()
        socket = null
        output = null
        executor.shutdownNow()
        writerExecutor.shutdownNow()
    }

    fun connectTo(rawAddress: String): Boolean {
        Log.i(TAG, "Manual LAN endpoint requested")
        val address = rawAddress.trim()
        val separator = address.lastIndexOf(':')
        if (separator <= 0) { Log.w(TAG, "Manual endpoint format invalid"); return false }
        val port = address.substring(separator + 1).toIntOrNull()?.takeIf { it in 1..65535 }
            ?: return false.also { Log.w(TAG, "Manual endpoint port invalid") }
        val host = runCatching { InetAddress.getByName(address.substring(0, separator)) }.getOrNull()
            ?: return false.also { Log.w(TAG, "Manual endpoint address invalid") }
        Log.i(TAG, "Manual endpoint accepted")
        manualEndpoint = true
        endpoint = host to port
        preferences.edit().putString(MANUAL_ENDPOINT_KEY, address).apply()
        writerExecutor.execute { runCatching { socket?.close() } }
        startWorker()
        return true
    }

    private fun startWorker() {
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

    private val resolver = object : NsdManager.ResolveListener {
        override fun onServiceResolved(info: NsdServiceInfo) {
            Log.i(TAG, "Handover LAN service resolved")
            if (!manualEndpoint) {
                endpoint = info.host to info.port
                startWorker()
            }
        }
        override fun onResolveFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
            Log.w(TAG, "LAN service resolution failed: $errorCode")
        }
    }

    private fun connectionLoop() {
        while (discovery != null) {
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
                ssl.soTimeout = 30_000
                ssl.startHandshake()
                socket = ssl
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
                    handle(message)
                }
            } catch (error: Exception) {
                Log.w(TAG, "LAN connection failed: ${error.javaClass.simpleName}")
                // Discovery remains active; retry the resolved endpoint after a bounded delay.
            } finally {
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
            if (discovery != null) try { Thread.sleep(RECONNECT_DELAY_MS) } catch (_: InterruptedException) { return }
        }
    }

    private fun handle(message: JSONObject) {
        if (message.optInt("protocol", -1) != 1) {
            socket?.close()
            return
        }
        when (message.optString("type")) {
            "hello" -> {
                val presentedFingerprint = peerFingerprint()
                val advertisedId = message.optString("id")
                if (presentedFingerprint == null || advertisedId != presentedFingerprint) {
                    socket?.close()
                    return
                }
                serverId = advertisedId.takeIf { it.isNotEmpty() }
                if (serverFingerprint != null) {
                    approvalGranted = true
                    return
                }
                // Unknown server: require a well-formed nonce commitment, then
                // reveal our own opening. The displayed code is derived after
                // both openings arrive, so it binds this ceremony rather than
                // only the long-lived certificates.
                val commit = message.optString("pair_commit")
                if (commit.isEmpty() || !isSha256Hex(commit)) {
                    socket?.close()
                    return
                }
                serverCommit = commit
                approvalGranted = false
                val nonce = ownNonce ?: return
                send(JSONObject().put("type", "pair_open").put("protocol", 1).put("nonce", nonce))
            }
            "pair_open" -> {
                if (serverFingerprint != null) return
                val commit = serverCommit
                val peer = serverId
                val nonce = ownNonce
                val serverNonce = message.optString("nonce")
                if (commit == null || peer == null || nonce == null
                    || !isPairingNonce(serverNonce) || pairingCommitment(serverNonce) != commit
                ) {
                    socket?.close()
                    return
                }
                val code = pairingCode(identity.deviceId, nonce, peer, serverNonce)
                pendingCode = code
                preferences.edit().putString(PENDING_CODE_KEY, code).apply()
                broadcast(ACTION_PAIR_REQUEST, JSONObject().put("code", code).put("server_id", peer))
            }
            "paired" -> {
                if (!approvalGranted && serverFingerprint == null) return
                val fingerprint = peerFingerprint() ?: return
                preferences.edit().putString(PIN_KEY, fingerprint).apply()
                serverFingerprint = fingerprint
                preferences.edit().remove(PENDING_CODE_KEY).apply()
                broadcast(ACTION_PAIRED, JSONObject().put("server_id", serverId))
                sendBattery()
                // Proactively share the current notification list; the server
                // also requests it, so a lost frame is recovered on request.
                HandoverNotificationService.snapshotFor(context).let { (enabled, list) ->
                    syncNotifications(enabled, list)
                }
                // Same recovery for media sessions.
                MediaObserver.activePushSync()
            }
            "revoke" -> {
                preferences.edit().remove(PIN_KEY).apply()
                serverFingerprint = null
                broadcast(ACTION_REVOKED, JSONObject())
                socket?.close()
            }
            "battery_request" -> sendBattery()
            "notifications_request" -> {
                if (serverFingerprint == null) return
                HandoverNotificationService.snapshotFor(context).let { (enabled, list) ->
                    syncNotifications(enabled, list)
                }
            }
            "notification_dismiss" -> {
                if (serverFingerprint == null) return
                HandoverNotificationService.dismiss(message.optString("key"))
            }
            "notification_reply" -> {
                if (serverFingerprint == null) return
                HandoverNotificationService.reply(message.optString("key"), message.optString("text"))
            }
            "notification_action" -> {
                if (serverFingerprint == null) return
                HandoverNotificationService.invokeAction(
                    message.optString("key"), message.optString("action_id"),
                )
            }
            "media_request" -> {
                if (serverFingerprint == null) return
                MediaObserver.activePushSync()
            }
            "media_control" -> {
                if (serverFingerprint == null) return
                val position = message.takeIf { it.has("position_ms") }?.optLong("position_ms")
                MediaObserver.executeControl(
                    message.optString("player"), message.optString("action"), position,
                )
            }
        }
    }

    private fun hello() = JSONObject().put("type", "hello").put("protocol", 1)
        .put("id", identity.deviceId).put("name", Build.MODEL ?: "Android device")
        .apply {
            serverFingerprint?.let { put("trusted_server_id", it) }
            ownNonce?.let { put("pair_commit", pairingCommitment(it)) }
        }

    private fun sendBattery() {
        val intent = context.registerReceiver(null, android.content.IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val reading = intent?.let(BatteryObserver::reading) ?: return
        send(JSONObject().put("type", "battery").put("protocol", 1)
            .put("percentage", reading.percentage).put("charging", reading.charging))
    }

    private fun writeNow(message: JSONObject) = synchronized(outputLock) {
        output?.let { stream -> runCatching { write(stream, message) }.onFailure { socket?.close() } }
    }
    private fun send(message: JSONObject) {
        if (!writerExecutor.isShutdown) writerExecutor.execute { writeNow(message) }
    }

    private fun sslContext(): SSLContext {
        val keyManagers = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm()).apply {
            init(identity.keyStore(), null)
        }.keyManagers
        val trust = pinnedTrustManager(serverFingerprint)
        return SSLContext.getInstance("TLSv1.3").apply { init(keyManagers, arrayOf(trust), SecureRandom()) }
    }

    private fun pinnedTrustManager(pin: String?): X509TrustManager = object : X509TrustManager {
        override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
        override fun checkClientTrusted(chain: Array<X509Certificate>, authType: String) = Unit
        override fun checkServerTrusted(chain: Array<X509Certificate>, authType: String) {
            if (pin != null && (chain.isEmpty() || chain[0].fingerprint() != pin)) throw java.security.cert.CertificateException("server certificate pin mismatch")
        }
    }

    private fun peerFingerprint(): String? = (socket?.session?.peerCertificates?.firstOrNull() as? X509Certificate)?.fingerprint()

    private fun write(output: BufferedOutputStream, message: JSONObject) {
        val bytes = message.toString().toByteArray(Charsets.UTF_8)
        require(bytes.size <= MAX_FRAME)
        output.write(ByteBuffer.allocate(4).putInt(bytes.size).array())
        output.write(bytes)
        output.flush()
    }

    private fun read(input: BufferedInputStream): JSONObject? {
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

    private fun broadcast(action: String, payload: JSONObject) {
        context.sendBroadcast(Intent(action).putExtra(EXTRA_JSON, payload.toString()).setPackage(context.packageName))
    }

    companion object {
        const val ACTION_PAIR_REQUEST = "org.handover.android.PAIR_REQUEST"
        const val ACTION_PAIRED = "org.handover.android.PAIRED"
        const val ACTION_REVOKED = "org.handover.android.REVOKED"
        const val EXTRA_JSON = "json"
        private const val SERVICE_TYPE = "_handover._tcp"
        private const val TAG = "HandoverNative"
        private const val PIN_KEY = "server_cert_sha256"
        private const val PENDING_CODE_KEY = "pending_pair_code"
        private const val MANUAL_ENDPOINT_KEY = "manual_endpoint"
        private const val MAX_FRAME = 64 * 1024
        private const val RECONNECT_DELAY_MS = 2_000L

        fun trustedPeerFingerprint(context: Context): String? =
            context.getSharedPreferences("handover_native_peers", Context.MODE_PRIVATE).getString(PIN_KEY, null)
        fun pendingPairingCode(context: Context): String? =
            context.getSharedPreferences("handover_native_peers", Context.MODE_PRIVATE).getString(PENDING_CODE_KEY, null)

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

        private fun String.hexBytes(): ByteArray {
            check(length % 2 == 0)
            return ByteArray(length / 2) { i -> substring(i * 2, i * 2 + 2).toInt(16).toByte() }
        }

        private fun isSha256Hex(value: String) =
            value.length == 64 && runCatching { value.hexBytes() }.isSuccess

        private fun isPairingNonce(value: String) =
            value.length == 32 && runCatching { value.hexBytes() }.isSuccess
    }
}
