package org.handover.android

import android.content.Context
import android.content.Intent
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import org.json.JSONObject
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.net.InetAddress
import java.net.Socket
import java.nio.ByteBuffer
import java.security.SecureRandom
import java.security.cert.X509Certificate
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import javax.net.ssl.TrustManager
import javax.net.ssl.X509TrustManager
import org.handover.android.DeviceIdentityStore.Companion.fingerprint

/** Native Handover transport. Frames are 4-byte big-endian length + UTF-8 JSON, max 64 KiB. */
class NativeTransport(private val context: Context) {
    private val identity = DeviceIdentityStore(context)
    private val nsd = context.getSystemService(NsdManager::class.java)
    private val executor: ExecutorService = Executors.newSingleThreadExecutor()
    private val preferences = context.getSharedPreferences("handover_native_peers", Context.MODE_PRIVATE)
    private var socket: SSLSocket? = null
    private var serverId: String? = null
    private var serverFingerprint: String? = null
    private var discovery: NsdManager.DiscoveryListener? = null

    fun start() {
        serverFingerprint = preferences.getString(PIN_KEY, null)
        discovery = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) = Unit
            override fun onDiscoveryStopped(serviceType: String) = Unit
            override fun onServiceFound(info: NsdServiceInfo) {
                if (info.serviceType == SERVICE_TYPE) nsd.resolveService(info, resolver)
            }
            override fun onServiceLost(serviceInfo: NsdServiceInfo) = Unit
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                runCatching { discovery?.let { nsd.stopServiceDiscovery(it) } }
            }
            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) = Unit
        }
        nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, discovery)
    }

    fun stop() {
        discovery?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        discovery = null
        socket?.close()
        socket = null
        executor.shutdownNow()
    }

    fun approvePair(code: String) = send(JSONObject().put("type", "pair_confirm").put("protocol", 1).put("code", code))

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

    private val resolver = object : NsdManager.ResolveListener {
        override fun onServiceResolved(info: NsdServiceInfo) {
            executor.execute { connect(info.host, info.port) }
        }
        override fun onResolveFailed(serviceInfo: NsdServiceInfo, errorCode: Int) = Unit
    }

    private fun connect(host: InetAddress, port: Int) {
        runCatching {
            socket?.close()
            val raw = Socket(host, port)
            val ssl = sslContext().socketFactory.createSocket(raw, host.hostAddress, port, true) as SSLSocket
            ssl.enabledProtocols = arrayOf("TLSv1.3")
            ssl.startHandshake()
            socket = ssl
            val input = BufferedInputStream(ssl.inputStream)
            val output = BufferedOutputStream(ssl.outputStream)
            write(output, hello())
            while (!ssl.isClosed) {
                val message = read(input) ?: break
                handle(message, output)
            }
        }
    }

    private fun hello() = JSONObject().put("type", "hello").put("protocol", 1)
        .put("id", identity.deviceId).put("name", Build.MODEL ?: "Android device")

    private fun handle(message: JSONObject, output: BufferedOutputStream) {
        when (message.optString("type")) {
            "hello" -> {
                val presentedFingerprint = peerFingerprint()
                val advertisedId = message.optString("id")
                if (presentedFingerprint == null || advertisedId != presentedFingerprint) {
                    socket?.close()
                    return
                }
                serverId = advertisedId.takeIf { it.isNotEmpty() }
                val code = pairingCode(identity.deviceId, serverId ?: return)
                broadcast(ACTION_PAIR_REQUEST, JSONObject().put("code", code).put("server_id", serverId))
            }
            "paired" -> {
                val fingerprint = peerFingerprint() ?: return
                preferences.edit().putString(PIN_KEY, fingerprint).apply()
                serverFingerprint = fingerprint
                broadcast(ACTION_PAIRED, JSONObject().put("server_id", serverId))
                sendBattery(output)
            }
            "revoke" -> {
                preferences.edit().remove(PIN_KEY).apply()
                serverFingerprint = null
                broadcast(ACTION_REVOKED, JSONObject())
            }
            "battery_request" -> sendBattery(output)
        }
    }

    private fun sendBattery(output: BufferedOutputStream) {
        val intent = context.registerReceiver(null, android.content.IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val reading = intent?.let(BatteryObserver::reading) ?: return
        write(output, JSONObject().put("type", "battery").put("protocol", 1)
            .put("percentage", reading.percentage).put("charging", reading.charging))
    }

    private fun send(message: JSONObject) = executor.execute { socket?.outputStream?.let {
        write(BufferedOutputStream(it), message)
    } }

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
        val header = input.readNBytes(4)
        if (header.size != 4) return null
        val size = ByteBuffer.wrap(header).int
        if (size !in 1..MAX_FRAME) return null
        val payload = input.readNBytes(size)
        if (payload.size != size) return null
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
        private const val PIN_KEY = "server_cert_sha256"
        private const val MAX_FRAME = 64 * 1024

        fun pairingCode(first: String, second: String): String {
            val low = minOf(first, second)
            val high = maxOf(first, second)
            val data = "handover-pair-v1:$low:$high".toByteArray(Charsets.UTF_8)
            val digest = java.security.MessageDigest.getInstance("SHA-256").digest(data)
            val value = ByteBuffer.wrap(digest.copyOfRange(0, 4)).int.toLong() and 0xffffffffL
            return "%08d".format(value % 100000000L)
        }
    }
}
