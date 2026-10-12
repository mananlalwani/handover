package org.handover.android

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.net.wifi.WifiManager
import android.os.Build
import android.util.Log
import org.webrtc.PeerConnection
import org.webrtc.PeerConnectionFactory
import org.webrtc.RtpTransceiver
import org.webrtc.SdpObserver
import org.webrtc.SessionDescription
import org.webrtc.MediaConstraints
import org.webrtc.MediaStreamTrack
import org.webrtc.audio.JavaAudioDeviceModule
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/** Receives a one-way laptop audio WebRTC offer. Signaling remains on NativeTransport. */
internal class LaptopRealtimeAudio(
    context: Context,
    private val sendAnswer: (id: String, sdp: String) -> Unit,
    private val playing: (id: String) -> Unit,
    private val stopped: (id: String) -> Unit,
) {
    private val appContext = context.applicationContext
    private var active: Session? = null

    private inner class Session(val id: String) {
        val executor = Executors.newSingleThreadExecutor { task ->
            Thread(task, "handover-webrtc-$id").apply { isDaemon = true }
        }
        val closed = AtomicBoolean(false)
        val answered = AtomicBoolean(false)
        val peerConnected = AtomicBoolean(false)
        val audioTrackStarted = AtomicBoolean(false)
        val reportedPlaying = AtomicBoolean(false)
        val reportedStopped = AtomicBoolean(false)
        var audioModule: JavaAudioDeviceModule? = null
        var factory: PeerConnectionFactory? = null
        var peer: PeerConnection? = null
        var wifiLock: WifiManager.WifiLock? = null
        var focusRequest: AudioFocusRequest? = null
        var focusHeld = false

        fun current(): Boolean = !closed.get() && synchronized(this@LaptopRealtimeAudio) { active === this }

        fun finish() {
            if (!closed.compareAndSet(false, true)) return
            synchronized(this@LaptopRealtimeAudio) {
                if (active === this) active = null
            }
            val cleanup = Runnable { dispose() }
            try {
                executor.execute(cleanup)
                executor.shutdown()
            } catch (_: Exception) {
                cleanup.run()
            }
            reportStopped()
        }

        fun reportStopped() {
            if (reportedStopped.compareAndSet(false, true)) runCatching { stopped(id) }
        }

        fun dispose() {
            runCatching { peer?.close() }
            runCatching { peer?.dispose() }
            peer = null
            runCatching { factory?.dispose() }
            factory = null
            runCatching { audioModule?.release() }
            audioModule = null
            val manager = appContext.getSystemService(AudioManager::class.java)
            runCatching { if (focusHeld && Build.VERSION.SDK_INT >= 26) focusRequest?.let(manager::abandonAudioFocusRequest) }
            focusHeld = false
            runCatching { if (wifiLock?.isHeld == true) wifiLock?.release() }
            wifiLock = null
        }

        fun start(offerSdp: String) {
            if (!current()) return
            if (offerSdp.isBlank() || offerSdp.length > MAX_SDP_CHARS) return finish()
            try {
                acquireResources() ?: return finish()
                check(current())
                synchronized(factoryLock) {
                    if (!factoryInitialized) {
                        PeerConnectionFactory.initialize(
                            PeerConnectionFactory.InitializationOptions.builder(appContext)
                                .createInitializationOptions(),
                        )
                        org.webrtc.Logging.enableLogToDebugOutput(org.webrtc.Logging.Severity.LS_NONE)
                        factoryInitialized = true
                    }
                }
                val module = JavaAudioDeviceModule.builder(appContext)
                    .setAudioAttributes(MEDIA_ATTRIBUTES)
                    .setUseStereoOutput(true)
                    .setUseLowLatency(true)
                    .setEnableVolumeLogger(false)
                    .setAudioTrackErrorCallback(object : JavaAudioDeviceModule.AudioTrackErrorCallback {
                        override fun onWebRtcAudioTrackInitError(error: String) = fail("audio output initialization failed")
                        override fun onWebRtcAudioTrackStartError(code: JavaAudioDeviceModule.AudioTrackStartErrorCode, error: String) = fail("audio output start failed")
                        override fun onWebRtcAudioTrackError(error: String) = fail("audio output failed")
                    })
                    .setAudioTrackStateCallback(object : JavaAudioDeviceModule.AudioTrackStateCallback {
                        override fun onWebRtcAudioTrackStart() {
                            audioTrackStarted.set(true)
                            maybeReportPlaying()
                        }

                        override fun onWebRtcAudioTrackStop() {
                            // A playout stop can be transient during route changes;
                            // report terminal stop only when this session is disposed.
                        }
                    })
                    .createAudioDeviceModule()
                audioModule = module
                check(current())
                val createdFactory = PeerConnectionFactory.builder()
                    .setAudioDeviceModule(module)
                    .createPeerConnectionFactory()
                factory = createdFactory
                val observer = object : PeerConnection.Observer {
                    override fun onSignalingChange(state: PeerConnection.SignalingState) = Unit
                    override fun onIceConnectionChange(state: PeerConnection.IceConnectionState) {
                        if (state == PeerConnection.IceConnectionState.FAILED ||
                            state == PeerConnection.IceConnectionState.CLOSED) finish()
                    }
                    override fun onIceConnectionReceivingChange(receiving: Boolean) = Unit
                    override fun onIceGatheringChange(state: PeerConnection.IceGatheringState) {
                        if (state == PeerConnection.IceGatheringState.COMPLETE) sendCompleteAnswer()
                    }
                    override fun onIceCandidate(candidate: org.webrtc.IceCandidate) = Unit
                    override fun onIceCandidatesRemoved(candidates: Array<out org.webrtc.IceCandidate>) = Unit
                    override fun onAddStream(stream: org.webrtc.MediaStream) = Unit
                    override fun onRemoveStream(stream: org.webrtc.MediaStream) = Unit
                    override fun onDataChannel(channel: org.webrtc.DataChannel) = Unit
                    override fun onRenegotiationNeeded() = Unit
                    override fun onAddTrack(receiver: org.webrtc.RtpReceiver, streams: Array<out org.webrtc.MediaStream>) = Unit
                    override fun onTrack(transceiver: RtpTransceiver) = Unit
                    override fun onConnectionChange(state: PeerConnection.PeerConnectionState) {
                        if (state == PeerConnection.PeerConnectionState.CONNECTED) {
                            peerConnected.set(true)
                            maybeReportPlaying()
                        } else if (state == PeerConnection.PeerConnectionState.DISCONNECTED) {
                            peerConnected.set(false)
                        }
                        if (state == PeerConnection.PeerConnectionState.FAILED ||
                            state == PeerConnection.PeerConnectionState.CLOSED) finish()
                    }
                    override fun onSelectedCandidatePairChanged(event: org.webrtc.CandidatePairChangeEvent) = Unit
                }
                val createdPeer = createdFactory.createPeerConnection(PeerConnection.RTCConfiguration(emptyList()), observer)
                    ?: error("PeerConnection unavailable")
                peer = createdPeer
                createdPeer.setRemoteDescription(object : SdpObserver {
                    override fun onCreateSuccess(description: SessionDescription) = Unit
                    override fun onSetSuccess() {
                        if (!current()) return
                        // No local audio source/track is created. Force every negotiated audio
                        // transceiver to receive-only before constructing the answer.
                        createdPeer.transceivers.forEach { transceiver ->
                            if (transceiver.mediaType == MediaStreamTrack.MediaType.MEDIA_TYPE_AUDIO) {
                                transceiver.setDirection(RtpTransceiver.RtpTransceiverDirection.RECV_ONLY)
                            }
                        }
                        createdPeer.createAnswer(object : SdpObserver {
                            override fun onCreateSuccess(description: SessionDescription) {
                                if (!current()) return
                                if (!answerIsAudioReceiveOnly(description.description)) {
                                    fail("answer would transmit audio")
                                    return
                                }
                                createdPeer.setLocalDescription(this, description)
                            }
                            override fun onSetSuccess() = Unit
                            override fun onCreateFailure(error: String) = fail("create answer failed")
                            override fun onSetFailure(error: String) = fail("set local description failed")
                        }, MediaConstraints())
                    }
                    override fun onCreateFailure(error: String) = Unit
                    override fun onSetFailure(error: String) = fail("set remote description failed")
                }, SessionDescription(SessionDescription.Type.OFFER, offerSdp))
            } catch (error: Exception) {
                Log.w(TAG, "WebRTC audio setup failed: ${error.javaClass.simpleName}")
                finish()
            }
        }

        private fun acquireResources(): Boolean {
            val wifi = appContext.getSystemService(WifiManager::class.java)
            wifiLock = runCatching {
                val mode = if (Build.VERSION.SDK_INT >= 29) WifiManager.WIFI_MODE_FULL_LOW_LATENCY
                    else WifiManager.WIFI_MODE_FULL_HIGH_PERF
                wifi.createWifiLock(mode, "Handover:realtime-audio").apply {
                    setReferenceCounted(false)
                    acquire()
                }
            }.getOrNull()
            val manager = appContext.getSystemService(AudioManager::class.java)
            if (Build.VERSION.SDK_INT >= 26) {
                val request = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                    .setAudioAttributes(MEDIA_ATTRIBUTES)
                    .setOnAudioFocusChangeListener { change -> if (change != AudioManager.AUDIOFOCUS_GAIN) finish() }
                    .build()
                focusRequest = request
                focusHeld = manager.requestAudioFocus(request) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
            } else {
                @Suppress("DEPRECATION")
                focusHeld = manager.requestAudioFocus(null, AudioManager.STREAM_MUSIC, AudioManager.AUDIOFOCUS_GAIN) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
            }
            return focusHeld
        }

        private fun sendCompleteAnswer() {
            if (!current() || !answered.compareAndSet(false, true)) return
            val sdp = peer?.localDescription?.description ?: return fail("missing local SDP")
            runCatching { sendAnswer(id, sdp) }.onFailure { fail("answer signaling failed") }
        }

        private fun maybeReportPlaying() {
            if (current() && peerConnected.get() && audioTrackStarted.get() &&
                reportedPlaying.compareAndSet(false, true)) {
                runCatching { playing(id) }
            }
        }

        private fun fail(reason: String) {
            Log.w(TAG, "WebRTC audio ended: $reason")
            finish()
        }
    }

    @Synchronized fun acceptOffer(id: String, sdp: String) {
        if (!NativeTransport.isTransferId(id)) return
        active?.finish()
        val session = Session(id)
        active = session
        session.executor.execute { session.start(sdp) }
    }

    @Synchronized fun stop(id: String? = null) {
        val session = active ?: return
        if (id != null && session.id != id) return
        session.finish()
    }

    /** Require an explicit receive-only/inactive direction on each audio m-line. */
    private fun answerIsAudioReceiveOnly(sdp: String): Boolean {
        var inAudio = false
        var sawAudio = false
        var direction: String? = null
        fun allowed(): Boolean = !inAudio || direction == "recvonly" || direction == "inactive"
        for (line in sdp.lineSequence()) {
            if (line.startsWith("m=")) {
                if (!allowed()) return false
                inAudio = line.startsWith("m=audio ")
                if (inAudio) {
                    sawAudio = true
                    direction = null
                }
            } else if (inAudio && line.startsWith("a=")) {
                when (line) {
                    "a=sendrecv" -> direction = "sendrecv"
                    "a=sendonly" -> direction = "sendonly"
                    "a=recvonly" -> direction = "recvonly"
                    "a=inactive" -> direction = "inactive"
                }
            }
        }
        return allowed() && sawAudio
    }

    companion object {
        private const val MAX_SDP_CHARS = 32 * 1024
        private const val TAG = "HandoverRealtimeAudio"
        private val MEDIA_ATTRIBUTES = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
            .build()
        @Volatile private var factoryInitialized = false
        private val factoryLock = Any()
    }
}
