package org.handover.android

import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
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
class NativeTransport(private val context: Context) {
    private val callObserver = CallObserver(context) { frame ->
        if (serverFingerprint != null) send(frame)
    }
    private val identity = DeviceIdentityStore(context)
    private val nsd = context.getSystemService(NsdManager::class.java)
    private val multicastLock = context.getSystemService(WifiManager::class.java)
        .createMulticastLock("handover-discovery").apply { setReferenceCounted(false) }
    private val executor: ExecutorService = Executors.newSingleThreadExecutor()
    private val writerExecutor = ThreadPoolExecutor(1, 1, 0, TimeUnit.MILLISECONDS,
        ArrayBlockingQueue(32)) { _, _ -> Thread { runCatching { socket?.close() } }.start() }
    private val transferExpiry = Executors.newSingleThreadScheduledExecutor()
    private val pendingTransfers = ConcurrentHashMap<String, PendingTransfer>()
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
        transferExpiry.scheduleWithFixedDelay({ expireTransfers() }, TRANSFER_SWEEP_MS,
            TRANSFER_SWEEP_MS, TimeUnit.MILLISECONDS)
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

    fun refreshCalls() = callObserver.refresh()

    fun stop() {
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

    fun requestContactsSync(): Boolean {
        if (serverFingerprint == null || context.checkSelfPermission("android.permission.READ_CONTACTS") !=
            android.content.pm.PackageManager.PERMISSION_GRANTED) return false
        val contacts = org.json.JSONArray()
        val projection = arrayOf(
            ContactsContract.Contacts._ID,
            ContactsContract.Contacts.DISPLAY_NAME,
            ContactsContract.Contacts.PHOTO_THUMBNAIL_URI,
        )
        context.contentResolver.query(
            ContactsContract.Contacts.CONTENT_URI, projection, null, null,
            ContactsContract.Contacts.DISPLAY_NAME + " COLLATE NOCASE ASC",
        )?.use { cursor ->
            val idIndex = cursor.getColumnIndexOrThrow(ContactsContract.Contacts._ID)
            val nameIndex = cursor.getColumnIndexOrThrow(ContactsContract.Contacts.DISPLAY_NAME)
            val photoIndex = cursor.getColumnIndexOrThrow(ContactsContract.Contacts.PHOTO_THUMBNAIL_URI)
            while (cursor.moveToNext()) {
                val id = cursor.getString(idIndex)
                val item = JSONObject().put("local_id", id)
                    .put("display_name", cursor.getString(nameIndex) ?: "")
                val phones = org.json.JSONArray()
                context.contentResolver.query(
                    ContactsContract.CommonDataKinds.Phone.CONTENT_URI,
                    arrayOf(ContactsContract.CommonDataKinds.Phone.NUMBER),
                    "${ContactsContract.CommonDataKinds.Phone.CONTACT_ID}=?", arrayOf(id), null,
                )?.use { phoneCursor ->
                    while (phoneCursor.moveToNext()) phones.put(phoneCursor.getString(0))
                }
                item.put("phones", phones).put("emails", org.json.JSONArray())
                val emails = org.json.JSONArray()
                context.contentResolver.query(
                    ContactsContract.CommonDataKinds.Email.CONTENT_URI,
                    arrayOf(ContactsContract.CommonDataKinds.Email.ADDRESS),
                    "${ContactsContract.CommonDataKinds.Email.CONTACT_ID}=?", arrayOf(id), null,
                )?.use { emailCursor ->
                    while (emailCursor.moveToNext()) {
                        emailCursor.getString(0)?.let { emails.put(it) }
                    }
                }
                item.put("emails", emails)
                val photoUri = cursor.getString(photoIndex)
                if (photoUri != null) {
                    val photo = encodeContactPhoto(Uri.parse(photoUri))
                    if (!photo.isNullOrEmpty() && contacts.toString().length + photo.length < 48 * 1024)
                        item.put("photo", photo)
                }
                contacts.put(item)
            }
        }
        send(JSONObject().put("type", "contacts_sync").put("protocol", 1).put("contacts", contacts))
        return true
    }

    private fun encodeContactPhoto(uri: Uri): String? = runCatching {
        val bitmap = context.contentResolver.openInputStream(uri)?.use(BitmapFactory::decodeStream)
            ?: return@runCatching null
        val size = maxOf(bitmap.width, bitmap.height)
        val scaled = if (size > 96) {
            val scale = 96f / size
            Bitmap.createScaledBitmap(
                bitmap, (bitmap.width * scale).toInt().coerceAtLeast(1),
                (bitmap.height * scale).toInt().coerceAtLeast(1), true,
            )
        } else bitmap
        ByteArrayOutputStream().use { output ->
            if (!scaled.compress(Bitmap.CompressFormat.JPEG, 70, output)) return@runCatching null
            Base64.encodeToString(output.toByteArray(), Base64.NO_WRAP)
        }.also {
            if (scaled !== bitmap) scaled.recycle()
            bitmap.recycle()
        }
    }.getOrNull()

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

    /** Sends a URL to the one explicitly paired desktop. */
    fun shareUrl(url: String): Boolean {
        if (serverFingerprint == null || socket?.isClosed != false || output == null || !validShareUrl(url)) return false
        val transferId = registerTransfer("url", url, Uri.parse(url)) ?: return false
        return if (enqueue {
            announceAccepted(transferId)
            writeNow(JSONObject().put("type", "share_url").put("protocol", 1)
                .put("transfer_id", transferId).put("url", url))
        }) true else {
            completeTransfer(transferId, "failed", TRANSFER_INTERRUPTED)
            false
        }
    }

    /**
     * Streams a content URI after its JSON header. The operation is serialized
     * with every other writer so raw bytes can never be mistaken for a frame.
     */
    fun shareFile(uri: Uri, requestedName: String? = null): Boolean {
        if (serverFingerprint == null || socket?.isClosed != false || output == null) return false
        val metadata = runCatching { fileMetadata(context.contentResolver, uri, requestedName) }.getOrNull() ?: return false
        val transferId = registerTransfer("file", metadata.first, uri) ?: return false
        return enqueue {
            announceAccepted(transferId)
            try {
                val resolver = context.contentResolver
                resolver.openInputStream(uri)?.use { input ->
                    writeNow(JSONObject().put("type", "share_file").put("protocol", 1)
                        .put("transfer_id", transferId).put("name", metadata.first).put("size", metadata.second))
                    val buffer = ByteArray(STREAM_BUFFER_BYTES)
                    var remaining = metadata.second
                    while (remaining > 0) {
                        val count = input.read(buffer, 0, minOf(buffer.size.toLong(), remaining).toInt())
                        if (count < 0) throw java.io.EOFException("file changed while sharing")
                        if (count == 0) continue
                        synchronized(outputLock) { output?.write(buffer, 0, count) ?: throw java.io.IOException("disconnected") }
                        remaining -= count
                    }
                    synchronized(outputLock) { output?.flush() ?: throw java.io.IOException("disconnected") }
                } ?: throw java.io.FileNotFoundException(uri.toString())
            } catch (error: Exception) {
                // A partial raw stream cannot be resynchronized as JSON. Drop
                // the authenticated session so the receiver deletes its temp.
                Log.w(TAG, "native file share failed: ${error.javaClass.simpleName}")
                completeTransfer(transferId, "failed", TRANSFER_INTERRUPTED)
                socket?.close()
            }
        }.also { accepted ->
            if (!accepted) completeTransfer(transferId, "failed", TRANSFER_INTERRUPTED)
        }
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
        broadcast(ACTION_CONNECTION_STATE, JSONObject().put("state", "reconnecting"))
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
            if (discovery != null) try { Thread.sleep(RECONNECT_DELAY_MS) } catch (_: InterruptedException) { return }
        }
    }

    private fun handle(message: JSONObject, input: BufferedInputStream) {
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
                publishConnectivity()
                // Proactively share the current notification list; the server
                // also requests it, so a lost frame is recovered on request.
                HandoverNotificationService.snapshotFor(context).let { (enabled, list) ->
                    syncNotifications(enabled, list)
                }
                callObserver.refresh()
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
            "ring" -> ringPhone()
            "user_ping" -> ringPhone()
            "notifications_request" -> {
                if (serverFingerprint == null) return
                HandoverNotificationService.snapshotFor(context).let { (enabled, list) ->
                    syncNotifications(enabled, list)
                }
            }
            "contacts_request" -> requestContactsSync()
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
            "remote_notification" -> {
                if (serverFingerprint == null) return
                HandoverNotificationService.postRemote(
                    message.optString("request_id"),
                    message.optString("app"),
                    message.optString("title"),
                    message.optString("body"),
                )
            }
            "call_request" -> {
                if (serverFingerprint == null) return
                callObserver.refresh()
            }
            "call_control" -> {
                if (serverFingerprint == null) return
                val requestId = message.optString("request_id")
                val action = message.optString("action")
                val requested = message.optLong("generation", -1L)
                val stale = requested != callObserver.generation
                val result = when (action) {
                    "place" -> if (stale) CallController.Result(false, "stale_state")
                        else CallController.place(context, message.optString("address"))
                    "answer" -> if (stale) CallController.Result(false, "stale_state")
                        else CallController.answer(context)
                    "decline" -> if (stale) CallController.Result(false, "stale_state")
                        else CallController.hangup(context, decline = true)
                    "hangup" -> if (stale) CallController.Result(false, "stale_state")
                        else CallController.hangup(context)
                    else -> CallController.Result(false, "rejected")
                }
                send(JSONObject().put("type", "call_result").put("protocol", 1)
                    .put("request_id", requestId).put("action", action)
                    .put("accepted", result.accepted).apply {
                        result.failure?.let { put("failure", it) }
                    })
                callObserver.refresh()
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
            "share_result" -> {
                if (serverFingerprint == null) return
                val transferId = message.optString("transfer_id")
                if (!isTransferId(transferId)) return
                val status = message.optString("status")
                if (status != "completed" && status != "failed") return
                val reason = message.optString("reason").takeIf { it.isNotEmpty() }
                if ((status == "completed" && reason != null) ||
                    (status == "failed" && reason !in TRANSFER_REASONS)) return
                completeTransfer(transferId, status, reason)
            }
            "share_url" -> {
                if (serverFingerprint == null) return
                val transferId = message.optString("transfer_id")
                val url = message.optString("url")
                if (!isTransferId(transferId) || !validShareUrl(url)) {
                    sendTransferResult(transferId, "failed", TRANSFER_INVALID_RESOURCE)
                    return
                }
                runCatching {
                    notifyReceived("url", url, url)
                    TransferHistory.add(context, "url", url, Uri.parse(url))
                    broadcast(ACTION_SHARE_RECEIVED, JSONObject().put("kind", "url").put("url", url)
                        .put("source", serverId ?: serverFingerprint))
                }.onSuccess { sendTransferResult(transferId, "completed", null) }
                    .onFailure { sendTransferResult(transferId, "failed", TRANSFER_STORAGE) }
            }
            "share_file" -> {
                if (serverFingerprint == null) return
                val transferId = message.optString("transfer_id")
                val name = safeFileName(message.optString("name")) ?: run {
                    sendTransferFailureAndClose(transferId, TRANSFER_INVALID_RESOURCE); return
                }
                val size = message.optLong("size", -1L)
                if (size !in 0..MAX_FILE_BYTES) {
                    sendTransferFailureAndClose(transferId, TRANSFER_SIZE_LIMIT); return
                }
                if (!isTransferId(transferId)) { socket?.close(); return }
                receiveFile(input, transferId, name, size)
            }
        }
    }

    private fun ringPhone() {
        if (serverFingerprint == null) return
        val ringtone = RingtoneManager.getRingtone(
            context,
            RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE),
        )
        ringtone?.play()
        Handler(Looper.getMainLooper()).postDelayed({ ringtone?.stop() }, 4_000)
        val vibrator = context.getSystemService(Vibrator::class.java)
        if (vibrator?.hasVibrator() == true) {
            val effect = VibrationEffect.createOneShot(1200, VibrationEffect.DEFAULT_AMPLITUDE)
            vibrator.vibrate(effect)
        }
    }

    private fun receiveFile(input: BufferedInputStream, transferId: String, name: String, size: Long) {
        if (Build.VERSION.SDK_INT >= 29) {
            receivePublicDownload(input, transferId, name, size)
            return
        }
        val root = File(context.getExternalFilesDir(android.os.Environment.DIRECTORY_DOWNLOADS), "Handover")
        if (!root.exists() && !root.mkdirs()) {
            sendTransferFailureAndClose(transferId, TRANSFER_STORAGE); return
        }
        val destination = uniqueDestination(root, name)
        val temporary = File(root, ".${name}.${java.util.UUID.randomUUID()}.part")
        try {
            FileOutputStream(temporary).use { outputStream ->
                val buffer = ByteArray(STREAM_BUFFER_BYTES)
                var remaining = size
                while (remaining > 0) {
                    val count = input.read(buffer, 0, minOf(buffer.size.toLong(), remaining).toInt())
                    if (count < 0) throw EOFException("interrupted file transfer")
                    if (count == 0) continue
                    outputStream.write(buffer, 0, count)
                    remaining -= count
                }
                outputStream.fd.sync()
            }
            try {
                Files.move(temporary.toPath(), destination.toPath(), StandardCopyOption.ATOMIC_MOVE)
            } catch (_: java.nio.file.AtomicMoveNotSupportedException) {
                Files.move(temporary.toPath(), destination.toPath())
            }
            runCatching { notifyReceived("file", name, null, null) }
            broadcast(ACTION_SHARE_RECEIVED, JSONObject().put("kind", "file").put("name", name)
                .put("path", destination.absolutePath).put("source", serverId ?: serverFingerprint))
            TransferHistory.add(context, "file", name, null)
            sendTransferResult(transferId, "completed", null)
        } catch (error: Exception) {
            temporary.delete()
            Log.w(TAG, "native file receive failed: ${error.javaClass.simpleName}")
            sendTransferFailureAndClose(transferId, if (error is EOFException) TRANSFER_INTERRUPTED else TRANSFER_STORAGE)
        }
    }

    @SuppressLint("NewApi")
    private fun receivePublicDownload(
        input: BufferedInputStream, transferId: String, name: String, size: Long,
    ) {
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, name)
            put(MediaStore.Downloads.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/Handover")
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val resolver = context.contentResolver
        val destination = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
        if (destination == null) {
            sendTransferFailureAndClose(transferId, TRANSFER_STORAGE)
            return
        }
        try {
            resolver.openOutputStream(destination, "w")!!.use { output ->
                val buffer = ByteArray(STREAM_BUFFER_BYTES)
                var remaining = size
                while (remaining > 0) {
                    val count = input.read(buffer, 0, minOf(buffer.size.toLong(), remaining).toInt())
                    if (count < 0) throw EOFException("interrupted file transfer")
                    if (count > 0) {
                        output.write(buffer, 0, count)
                        remaining -= count
                    }
                }
            }
            resolver.update(destination, ContentValues().apply {
                put(MediaStore.Downloads.IS_PENDING, 0)
            }, null, null)
            notifyReceived("file", name, null, destination)
            broadcast(ACTION_SHARE_RECEIVED, JSONObject().put("kind", "file").put("name", name)
                .put("path", destination.toString()).put("source", serverId ?: serverFingerprint))
            TransferHistory.add(context, "file", name, destination)
            sendTransferResult(transferId, "completed", null)
        } catch (error: Exception) {
            resolver.delete(destination, null, null)
            Log.w(TAG, "native public download failed: ${error.javaClass.simpleName}")
            sendTransferFailureAndClose(
                transferId, if (error is EOFException) TRANSFER_INTERRUPTED else TRANSFER_STORAGE,
            )
        }
    }

    private fun notifyReceived(kind: String, value: String, url: String?, file: Uri? = null) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(SHARE_CHANNEL, "Received shares", NotificationManager.IMPORTANCE_DEFAULT))
        val source = (serverId ?: serverFingerprint ?: "unknown desktop").take(12)
        val builder = android.app.Notification.Builder(context, SHARE_CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setAutoCancel(true)
        if (kind == "url") {
            builder.setContentTitle("URL received from $source")
                .setContentText("Tap to open the received URL")
            val parsed = url?.let { Uri.parse(it) }
            if (parsed != null && parsed.scheme?.lowercase() in setOf("http", "https")) {
                val intent = Intent(Intent.ACTION_VIEW, parsed)
                builder.setContentIntent(PendingIntent.getActivity(context, 0, intent,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
            }
        } else {
            val update = file?.let { AppUpdater.inspectAndRemember(context, it) }
            builder.setContentTitle(if (update != null) "Handover update ready" else "File received from $source")
                .setContentText(if (update != null) "Version ${update.versionName.ifEmpty { update.versionCode.toString() }} · tap to install" else value)
            if (file != null) {
                val intent = if (update != null) AppUpdater.installIntent(context, update) else {
                    val mime = context.contentResolver.getType(file)
                        ?: java.net.URLConnection.guessContentTypeFromName(value)
                        ?: "application/octet-stream"
                    Intent(Intent.ACTION_VIEW).setDataAndType(file, mime)
                        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                }
                builder.setContentIntent(PendingIntent.getActivity(
                    context, file.hashCode(), intent,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                ))
            }
        }
        manager.notify((System.currentTimeMillis() and 0x7fffffff).toInt(), builder.build())
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

    private fun sendTransferResult(transferId: String, status: String, reason: String?) {
        if (!isTransferId(transferId)) return
        val result = JSONObject().put("type", "share_result").put("protocol", 1)
            .put("transfer_id", transferId).put("status", status)
        reason?.let { result.put("reason", it) }
        if (!enqueue { writeNow(result) }) socket?.close()
    }

    private fun sendTransferFailureAndClose(transferId: String, reason: String) {
        if (isTransferId(transferId)) {
            val result = JSONObject().put("type", "share_result").put("protocol", 1)
                .put("transfer_id", transferId).put("status", "failed").put("reason", reason)
            runCatching { writerExecutor.submit { writeNow(result) }.get(2, TimeUnit.SECONDS) }
        }
        socket?.close()
    }

    private fun registerTransfer(kind: String, name: String, uri: Uri?): String? {
        if (pendingTransfers.size >= MAX_PENDING_TRANSFERS) return null
        val id = UUID.randomUUID().toString().replace("-", "")
        pendingTransfers[id] = PendingTransfer(
            android.os.SystemClock.elapsedRealtime() + TRANSFER_TIMEOUT_MS, kind, name, uri,
        )
        return id
    }

    private fun announceAccepted(id: String) {
        val transfer = pendingTransfers[id] ?: return
        publishTransferResult(transfer.result(id, "accepted"))
    }

    private fun completeTransfer(id: String, status: String, reason: String?) {
        val transfer = pendingTransfers.remove(id) ?: return
        val result = transfer.result(id, status)
        reason?.let { result.put("reason", it) }
        publishTransferResult(result)
    }

    private fun publishTransferResult(result: JSONObject) {
        TransferHistory.recordResult(
            context,
            result.optString("transfer_id"),
            result.optString("status"),
            result.optString("kind").takeIf(String::isNotEmpty),
            result.optString("name").takeIf(String::isNotEmpty),
            result.optString("uri").takeIf(String::isNotEmpty)?.let(Uri::parse),
        )
        broadcast(ACTION_TRANSFER_RESULT, result)
    }

    private fun expireTransfers() {
        val now = android.os.SystemClock.elapsedRealtime()
        val expired = pendingTransfers.entries.filter { it.value.deadline <= now }
        expired.forEach {
            completeTransfer(it.key, "failed", TRANSFER_TIMED_OUT)
        }
        if (expired.isNotEmpty()) socket?.close()
    }

    private fun failPendingTransfers(reason: String) {
        pendingTransfers.keys.toList().forEach { completeTransfer(it, "failed", reason) }
    }

    private data class PendingTransfer(
        val deadline: Long,
        val kind: String,
        val name: String,
        val uri: Uri?,
    ) {
        fun result(id: String, status: String) = JSONObject()
            .put("transfer_id", id).put("status", status).put("kind", kind).put("name", name)
            .apply { uri?.let { put("uri", it.toString()) } }
    }

    private fun send(message: JSONObject) {
        enqueue { writeNow(message) }
    }

    private fun enqueue(operation: () -> Unit): Boolean = try {
        if (writerExecutor.isShutdown) false else { writerExecutor.execute(operation); true }
    } catch (_: java.util.concurrent.RejectedExecutionException) { false }

    private fun sslContext(): SSLContext {
        val keyManagers = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm()).apply {
            init(identity.keyStore(), null)
        }.keyManagers
        val trust = pinnedTrustManager(serverFingerprint)
        return SSLContext.getInstance("TLSv1.3").apply { init(keyManagers, arrayOf(trust), SecureRandom()) }
    }

    private fun pinnedTrustManager(pin: String?): X509TrustManager =
        // This is deliberately custom: the first pairing handshake has no CA
        // trust anchor. It accepts only a non-empty, currently valid leaf for
        // the user-visible pairing ceremony; every subsequent handshake is
        // restricted to the stored certificate fingerprint. It never accepts
        // an empty chain and never disables TLS verification for a paired peer.
        PinnedIdentityTrustManager(pin)

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
        const val ACTION_CONNECTION_STATE = "org.handover.android.CONNECTION_STATE"
        const val EXTRA_JSON = "json"
        private const val SERVICE_TYPE = "_handover._tcp"
        private const val TAG = "HandoverNative"
        private const val PIN_KEY = "server_cert_sha256"
        private const val PENDING_CODE_KEY = "pending_pair_code"
        private const val MANUAL_ENDPOINT_KEY = "manual_endpoint"
        private const val MAX_FRAME = 64 * 1024
        private const val MAX_FILE_BYTES = 100L * 1024 * 1024
        private const val MAX_URL_BYTES = 8 * 1024
        private const val MAX_NAME_BYTES = 255
        private const val STREAM_BUFFER_BYTES = 32 * 1024
        private const val SHARE_CHANNEL = "handover_received_shares"
        private const val RECONNECT_DELAY_MS = 2_000L

        const val ACTION_SHARE_RECEIVED = "org.handover.android.SHARE_RECEIVED"
        const val ACTION_TRANSFER_RESULT = "org.handover.android.TRANSFER_RESULT"
        const val EXTRA_SHARE_URI = "uri"
        const val EXTRA_SHARE_TEXT = "text"
        private const val MAX_PENDING_TRANSFERS = 32
        private const val TRANSFER_TIMEOUT_MS = 120_000L
        private const val TRANSFER_SWEEP_MS = 5_000L
        private const val TRANSFER_INVALID_RESOURCE = "invalid_resource"
        private const val TRANSFER_SIZE_LIMIT = "size_limit"
        private const val TRANSFER_STORAGE = "storage"
        private const val TRANSFER_INTERRUPTED = "interrupted"
        private const val TRANSFER_TIMED_OUT = "timed_out"
        private const val TRANSFER_DISCONNECTED = "disconnected"
        private const val TRANSFER_REJECTED = "rejected"
        private val TRANSFER_REASONS = setOf("invalid_resource", "size_limit", "storage",
            "interrupted", "rejected", "timed_out", "disconnected", "transport")

        fun trustedPeerFingerprint(context: Context): String? =
            context.getSharedPreferences("handover_native_peers", Context.MODE_PRIVATE).getString(PIN_KEY, null)
        fun pendingPairingCode(context: Context): String? =
            context.getSharedPreferences("handover_native_peers", Context.MODE_PRIVATE).getString(PENDING_CODE_KEY, null)

        private fun fileMetadata(resolver: ContentResolver, uri: Uri, requestedName: String?): Pair<String, Long>? {
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

        private fun safeFileName(raw: String): String? {
            val basename = raw.replace('\\', '/').substringAfterLast('/').trim()
            if (basename.isEmpty() || basename == "." || basename == ".." || basename.any { it.code < 0x20 || it == '\u0000' }) return null
            val bytes = basename.toByteArray(Charsets.UTF_8)
            if (bytes.size <= MAX_NAME_BYTES) return basename
            var end = MAX_NAME_BYTES
            while (end > 0 && (bytes[end].toInt() and 0xc0) == 0x80) end--
            return String(bytes, 0, end, Charsets.UTF_8).ifEmpty { null }
        }

        private fun validShareUrl(value: String): Boolean {
            if (value.isEmpty() || value.toByteArray(Charsets.UTF_8).size > MAX_URL_BYTES ||
                value.trim() != value || value.any { it.isISOControl() || it.isWhitespace() }) return false
            val parsed = runCatching { java.net.URI(value) }.getOrNull() ?: return false
            return !parsed.scheme.isNullOrBlank() &&
                parsed.scheme.lowercase() !in setOf("file", "javascript", "data")
        }

        private fun isTransferId(value: String): Boolean =
            value.length == 32 && value.all { it in '0'..'9' || it in 'a'..'f' }

        private fun uniqueDestination(root: File, name: String): File {
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
