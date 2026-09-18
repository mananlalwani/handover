package org.handover.android

import android.annotation.SuppressLint
import android.content.ComponentName
import android.content.Context
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.os.Handler
import android.os.Looper
import android.util.Log
import java.lang.ref.WeakReference
import org.json.JSONObject

/** Phone-side media state for the native backend.
 *
 * Sessions come from the platform [MediaSessionManager]; no extra permission
 * is needed because registration uses this app's enabled notification-listener
 * component. Transport commands arriving after pairing drive the matching
 * controller's transport controls. Normal clients never see which backend
 * produced a session: the daemon normalizes these into the shared model.
 */
data class WireMediaSession(
    val player: String,
    val application: String,
    val title: String?,
    val artist: String?,
    val album: String?,
    val playback: String,
    val positionMs: Long?,
    val durationMs: Long?,
    val controls: List<String>,
)

/** Event-driven observer owned by the foreground service. */
class MediaObserver(private val context: Context) {
    private val appContext = context.applicationContext
    private val manager = appContext.getSystemService(MediaSessionManager::class.java)
    private val mainHandler = Handler(Looper.getMainLooper())
    private val controllers = mutableMapOf<String, MediaController>()
    private val callbacks = mutableMapOf<String, MediaController.Callback>()
    private val listenerComponent =
        ComponentName(appContext, HandoverNotificationService::class.java)

    private val sessionsListener =
        MediaSessionManager.OnActiveSessionsChangedListener { active ->
            resync(active ?: emptyList())
        }

    fun start() {
        activeInstance = WeakReference(this)
        try {
            manager.addOnActiveSessionsChangedListener(sessionsListener, listenerComponent)
        } catch (error: SecurityException) {
            // Listener access revoked: report nothing. The daemon only gains
            // the media capability from actual session reports, and a revoked
            // listener clears native media via the disabled notifications sync.
            Log.w(TAG, "Media observation unavailable without listener access")
            return
        }
        resync(runCatching { manager.getActiveSessions(listenerComponent) }.getOrDefault(emptyList()))
    }

    fun stop() {
        if (activeInstance?.get() === this) activeInstance = null
        runCatching { manager.removeOnActiveSessionsChangedListener(sessionsListener) }
        synchronized(controllers) {
            controllers.forEach { (player, controller) ->
                callbacks[player]?.let { runCatching { controller.unregisterCallback(it) } }
            }
            controllers.clear()
            callbacks.clear()
        }
    }

    /** Push the full current list; called after pairing and on server request. */
    fun pushSync() {
        val list = snapshot()
        transport?.syncMedia(list)
    }

    private fun snapshot(): List<WireMediaSession> = synchronized(controllers) {
        controllers.values.mapNotNull { snapshotOf(appContext, it) }
    }

    private fun resync(active: List<MediaController>) {
        val fresh = active.mapNotNull { controller ->
            val player = controller.packageName?.takeIf { isValidPlayer(it) } ?: return@mapNotNull null
            player to controller
        }.toMap()
        val removed: List<String>
        synchronized(controllers) {
            removed = controllers.keys.filter { it !in fresh }
            removed.forEach { player ->
                val controller = controllers[player]
                val callback = callbacks[player]
                if (controller != null && callback != null) {
                    runCatching { controller.unregisterCallback(callback) }
                }
                controllers.remove(player)
                callbacks.remove(player)
            }
            fresh.forEach { (player, controller) ->
                if (player !in controllers) {
                    val callback = object : MediaController.Callback() {
                        override fun onPlaybackStateChanged(state: PlaybackState?) {
                            snapshotOf(appContext, controller)?.let {
                                transport?.publishMedia(it)
                            }
                        }

                        override fun onMetadataChanged(metadata: MediaMetadata?) {
                            snapshotOf(appContext, controller)?.let {
                                transport?.publishMedia(it)
                            }
                        }

                        override fun onSessionDestroyed() {
                            synchronized(controllers) {
                                callbacks[player]?.let { registered ->
                                    runCatching { controller.unregisterCallback(registered) }
                                }
                                controllers.remove(player)
                                callbacks.remove(player)
                            }
                            transport?.retractMedia(player)
                        }
                    }
                    controller.registerCallback(callback, mainHandler)
                    controllers[player] = controller
                    callbacks[player] = callback
                }
            }
        }
        removed.forEach { transport?.retractMedia(it) }
        // Authoritative list: reconcile anything the deltas may have missed.
        val list = synchronized(controllers) {
            controllers.values.mapNotNull { snapshotOf(appContext, it) }
        }
        // Never log titles or artists; counts only.
        Log.i(TAG, "Media sync: ${list.size} active")
        transport?.syncMedia(list)
    }

    companion object {
        private const val TAG = "HandoverMedia"
        @Volatile var transport: NativeTransport? = null
        @Volatile private var activeInstance: WeakReference<MediaObserver>? = null

        /** Push a sync from the current observer, if one is running. */
        fun activePushSync() {
            activeInstance?.get()?.pushSync()
        }

        const val MAX_PLAYER = 128
        const val MAX_APP = 128
        const val MAX_TEXT = 512

        fun isValidPlayer(player: String): Boolean =
            player.isNotEmpty() && player.length <= MAX_PLAYER && player.none { it.isISOControl() }

        fun truncateTo(value: String, max: Int): String =
            if (value.length <= max) value else value.substring(0, max)

        /** Map platform playback states without inventing precision.
         * Only actively playing or paused states report as such; transitional
         * states (buffering, connecting, skipping, fast-forward/rewind) and
         * errors report unknown rather than guessing. */
        fun playbackOf(state: Int): String = when (state) {
            PlaybackState.STATE_PLAYING -> "playing"
            PlaybackState.STATE_PAUSED -> "paused"
            PlaybackState.STATE_STOPPED -> "stopped"
            else -> "unknown"
        }

        /** Advertise exactly the controls behind the platform actions bitmask.
         * Absolute `seekTo` maps to `set_position`; relative seeks have no
         * genuine platform API and are never advertised. A combined toggle is
         * derived when both discrete bits exist, mirroring the KDE adapter so
         * clients see one policy from either backend. */
        fun controlsOf(actions: Long): List<String> = buildList {
            if (actions and PlaybackState.ACTION_PLAY != 0L) add("play")
            if (actions and PlaybackState.ACTION_PAUSE != 0L) add("pause")
            if (actions and PlaybackState.ACTION_PLAY_PAUSE != 0L
                || (actions and PlaybackState.ACTION_PLAY != 0L
                    && actions and PlaybackState.ACTION_PAUSE != 0L)
            ) {
                add("play_pause")
            }
            if (actions and PlaybackState.ACTION_SKIP_TO_NEXT != 0L) add("next")
            if (actions and PlaybackState.ACTION_SKIP_TO_PREVIOUS != 0L) add("previous")
            if (actions and PlaybackState.ACTION_SEEK_TO != 0L) add("set_position")
        }

        fun isValidCommand(action: String, positionMs: Long?): Boolean {
            if (action !in setOf("play", "pause", "play_pause", "next", "previous", "set_position")) {
                return false
            }
            if (action == "set_position" && (positionMs == null || positionMs < 0)) return false
            return true
        }

        /** Map one platform controller into the wire model. Null when unusable. */
        fun snapshotOf(context: Context, controller: MediaController): WireMediaSession? {
            val player = controller.packageName ?: return null
            if (!isValidPlayer(player)) return null
            val app = truncateTo(appName(context, player), MAX_APP)
            val metadata = controller.metadata
            val state = controller.playbackState
            val actions = state?.actions ?: 0L
            return WireMediaSession(
                player = player,
                application = app,
                title = metadata?.getString(MediaMetadata.METADATA_KEY_TITLE)
                    ?.ifBlank { null }?.let { truncateTo(it, MAX_TEXT) },
                artist = metadata?.getString(MediaMetadata.METADATA_KEY_ARTIST)
                    ?.ifBlank { null }?.let { truncateTo(it, MAX_TEXT) },
                album = metadata?.getString(MediaMetadata.METADATA_KEY_ALBUM)
                    ?.ifBlank { null }?.let { truncateTo(it, MAX_TEXT) },
                playback = playbackOf(state?.state ?: PlaybackState.STATE_NONE),
                positionMs = state?.position?.takeIf { it >= 0 },
                durationMs = metadata?.getLong(MediaMetadata.METADATA_KEY_DURATION)?.takeIf { it > 0 },
                controls = controlsOf(actions),
            )
        }

        private fun appName(context: Context, packageName: String): String {
            return runCatching {
                val pm = context.packageManager
                val info = pm.getApplicationInfo(packageName, 0)
                pm.getApplicationLabel(info).toString().ifEmpty { packageName }
            }.getOrDefault(packageName)
        }

        /** Execute one validated server command on the matching controller. */
        fun executeControl(player: String, action: String, positionMs: Long?): Boolean {
            if (!isValidPlayer(player) || !isValidCommand(action, positionMs)) return false
            val observer = activeInstance?.get() ?: return false
            val controller = synchronized(observer.controllers) {
                observer.controllers[player]
            } ?: return false
            // Only drive controls the platform reported; the daemon already
            // capability-gates, so this is defense in depth.
            val reported = controlsOf(controller.playbackState?.actions ?: 0L)
            val effective = if (action == "play_pause" && action !in reported) {
                when (controller.playbackState?.state) {
                    PlaybackState.STATE_PLAYING -> "pause"
                    else -> "play"
                }.takeIf { it in reported } ?: return false
            } else {
                if (action !in reported) return false
                action
            }
            return runCatching {
                val transportControls = controller.transportControls
                when (effective) {
                    "play" -> transportControls.play()
                    "pause" -> transportControls.pause()
                    "play_pause" -> {
                        // No platform toggle exists; drive it from live state.
                        if (controller.playbackState?.state == PlaybackState.STATE_PLAYING) {
                            transportControls.pause()
                        } else {
                            transportControls.play()
                        }
                    }
                    "next" -> transportControls.skipToNext()
                    "previous" -> transportControls.skipToPrevious()
                    else -> transportControls.seekTo(positionMs ?: return false)
                }
                true
            }.getOrDefault(false)
        }

        fun postJson(session: WireMediaSession): JSONObject =
            JSONObject(postMap(session))

        /** Pure map form of [postJson] for unit tests. */
        fun postMap(session: WireMediaSession): Map<String, Any?> = mapOf(
            "type" to "media_post",
            "protocol" to 1,
            "player" to session.player,
            "application" to session.application,
            "title" to session.title,
            "artist" to session.artist,
            "album" to session.album,
            "playback" to session.playback,
            "position_ms" to session.positionMs,
            "duration_ms" to session.durationMs,
            "controls" to session.controls,
        )

        fun syncJson(sessions: List<WireMediaSession>): JSONObject =
            JSONObject(syncMap(sessions))

        /** Pure map form of [syncJson] for unit tests. */
        fun syncMap(sessions: List<WireMediaSession>): Map<String, Any?> = mapOf(
            "type" to "media_sync",
            "protocol" to 1,
            "sessions" to sessions.map {
                mapOf(
                    "player" to it.player,
                    "application" to it.application,
                    "title" to it.title,
                    "artist" to it.artist,
                    "album" to it.album,
                    "playback" to it.playback,
                    "position_ms" to it.positionMs,
                    "duration_ms" to it.durationMs,
                    "controls" to it.controls,
                )
            },
        )

        fun controlJson(player: String, action: String, positionMs: Long?): JSONObject =
            JSONObject(controlMap(player, action, positionMs))

        /** Pure map form of [controlJson] for unit tests. */
        fun controlMap(player: String, action: String, positionMs: Long?): Map<String, Any?> =
            mapOf(
                "type" to "media_control",
                "protocol" to 1,
                "player" to player,
                "action" to action,
                "position_ms" to positionMs,
            )
    }
}
