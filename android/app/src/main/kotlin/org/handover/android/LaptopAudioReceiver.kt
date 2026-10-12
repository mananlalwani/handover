package org.handover.android

import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import android.os.Process
import org.json.JSONObject
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.DataInputStream
import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit
import javax.net.ssl.SSLSocket

/** Fixed wire format: 8-byte monotonic microseconds followed by 1920 PCM bytes.
 * PCM is stereo signed 16-bit little endian, 48000 Hz, 480 frames per block.
 * TLS reuses the paired identities; control/file traffic uses another socket.
 */
internal class LaptopAudioReceiver(private val transport: NativeTransport) {
    private class Session(val id: String) {
        @Volatile var running = true
        @Volatile var socket: Socket? = null
        val queue = ArrayBlockingQueue<ByteArray>(6)
        val overflow = java.util.concurrent.atomic.AtomicBoolean(false)
        fun stop() {
            running = false
            runCatching { socket?.close() }
            queue.clear()
        }
    }
    private var session: Session? = null

    @Synchronized fun start(id: String) {
        if (!NativeTransport.isTransferId(id) || transport.serverFingerprint == null) return
        session?.stop()
        val next = Session(id)
        session = next
        Thread({ receive(next) }, "handover-audio-network").start()
    }

    @Synchronized fun stop(id: String? = null) {
        if (id != null && session?.id != id) return
        session?.stop()
        session = null
    }

    private fun receive(current: Session) {
        var playback: Thread? = null
        var wifiLock: android.net.wifi.WifiManager.WifiLock? = null
        try {
            // Hold this only for the audio connection; eligibility is owned
            // by Android and depends on device, screen and foreground state.
            wifiLock = runCatching {
                val manager = transport.context.getSystemService(android.net.wifi.WifiManager::class.java)
                @Suppress("DEPRECATION")
                val mode = if (android.os.Build.VERSION.SDK_INT >= 29)
                    android.net.wifi.WifiManager.WIFI_MODE_FULL_LOW_LATENCY
                    else android.net.wifi.WifiManager.WIFI_MODE_FULL_HIGH_PERF
                manager.createWifiLock(mode, "Handover:audio").apply {
                    setReferenceCounted(false)
                    acquire()
                }
            }.getOrNull()
            val control = transport.socket ?: error("offline")
            val pin = transport.serverFingerprint ?: error("unpaired")
            check(current.running)
            val raw = Socket()
            current.socket = raw
            check(current.running)
            raw.tcpNoDelay = true
            raw.connect(InetSocketAddress(control.inetAddress, control.port), 5000)
            val tls = transport.sslContext(pin).socketFactory.createSocket(
                raw, control.inetAddress.hostAddress, control.port, true) as SSLSocket
            current.socket = tls
            check(current.running)
            tls.enabledProtocols = arrayOf("TLSv1.3")
            tls.soTimeout = 5000
            tls.startHandshake()
            check(current.running && transport.socket === control && transport.serverFingerprint == pin)
            transport.write(BufferedOutputStream(tls.outputStream), JSONObject()
                .put("type", "audio_receive").put("protocol", 1).put("stream_id", current.id))
            val input = BufferedInputStream(tls.inputStream, 4096)
            val format = transport.read(input) ?: error("missing format")
            check(format.optInt("protocol") == 1 && format.optString("type") == "audio_format")
            tls.soTimeout = 2000
            playback = Thread({ play(current) }, "handover-audio-playback").apply { start() }
            val data = DataInputStream(input)
            var base: Long? = null
            var baselineEpoch = System.nanoTime() / 1000
            var previous = -1L
            while (current.running) {
                val sent = data.readLong()
                check(sent >= 0 && sent >= previous)
                previous = sent
                val block = ByteArray(1920)
                data.readFully(block)
                val arrival = System.nanoTime() / 1000
                // Compare clock progress, never wall clocks across devices.
                val offset = arrival - sent
                if (arrival - baselineEpoch >= 10_000_000) {
                    base = offset
                    baselineEpoch = arrival
                }
                base = minOf(base ?: offset, offset)
                if (offset - base > 60_000) continue
                if (!current.queue.offer(block)) {
                    current.queue.poll()
                    current.queue.offer(block)
                    current.overflow.set(true)
                }
            }
        } catch (_: Exception) {
            // Payloads and connection credentials never enter logs.
        } finally {
            current.stop()
            runCatching { if (wifiLock?.isHeld == true) wifiLock?.release() }
            HandoverForegroundService.audioStopped(current.id)
            playback?.join(2000)
            // Stamp the result so an old worker cannot stop a newer session.
            transport.send(JSONObject().put("type", "audio_stop").put("protocol", 1)
                .put("stream_id", current.id))
        }
    }

    private fun play(current: Session) {
        Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO)
        val manager = transport.context.getSystemService(AudioManager::class.java)
        val attributes = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build()
        val focus = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
            .setAudioAttributes(attributes).setOnAudioFocusChangeListener { change ->
                if (change != AudioManager.AUDIOFOCUS_GAIN) current.stop()
            }.build()
        var track: AudioTrack? = null
        var focusHeld = false
        try {
            focusHeld = manager.requestAudioFocus(focus) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
            check(focusHeld && current.running)
            val minimum = AudioTrack.getMinBufferSize(48000, AudioFormat.CHANNEL_OUT_STEREO,
                AudioFormat.ENCODING_PCM_16BIT)
            check(minimum > 0)
            val output = AudioTrack.Builder().setAudioAttributes(attributes)
                .setAudioFormat(AudioFormat.Builder().setSampleRate(48000)
                    .setChannelMask(AudioFormat.CHANNEL_OUT_STEREO)
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT).build())
                .setTransferMode(AudioTrack.MODE_STREAM)
                .setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
                .setBufferSizeInBytes(maxOf(minimum * 2, 15360)).build()
            track = output
            check(output.state == AudioTrack.STATE_INITIALIZED)
            output.setBufferSizeInFrames(1920)
            val prefillFrames = minOf(output.bufferSizeInFrames, 1920)
            var prefilledFrames = 0
            var previousUnderruns = 0
            var lastBufferGrowth = 0L
            var started = false
            while (current.running) {
                var block = current.queue.poll(500, TimeUnit.MILLISECONDS) ?: continue
                // A normal packet burst is not an overflow. Only discard
                // buffered playback when the bounded receive queue lost data.
                if (current.overflow.getAndSet(false)) {
                    while (true) block = current.queue.poll() ?: break
                    if (started) output.pause()
                    output.flush()
                    prefilledFrames = 0
                    started = false
                }
                var offset = 0
                while (offset < block.size && current.running) {
                    val written = output.write(block, offset, block.size - offset, AudioTrack.WRITE_NON_BLOCKING)
                    check(written >= 0)
                    if (written == 0) Thread.sleep(2) else {
                        offset += written
                        prefilledFrames += written / 4
                    }
                    if (!started && current.running && prefilledFrames >= prefillFrames) {
                        output.play()
                        started = true
                        HandoverForegroundService.audioStarted(current.id)
                        transport.send(JSONObject().put("type", "audio_playing").put("protocol", 1)
                            .put("stream_id", current.id))
                    }
                }
                val underruns = output.underrunCount
                val now = android.os.SystemClock.elapsedRealtime()
                if (started && underruns > previousUnderruns && now - lastBufferGrowth >= 1000) {
                    output.setBufferSizeInFrames(minOf(output.bufferCapacityInFrames,
                        minOf(3840, output.bufferSizeInFrames + 480)))
                    lastBufferGrowth = now
                }
                previousUnderruns = underruns
            }
        } catch (_: Exception) {
        } finally {
            current.stop()
            runCatching { track?.pause() }
            runCatching { track?.flush() }
            track?.release()
            if (focusHeld) manager.abandonAudioFocusRequest(focus)
        }
    }
}
