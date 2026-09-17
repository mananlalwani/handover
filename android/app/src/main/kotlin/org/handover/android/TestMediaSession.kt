package org.handover.android

import android.content.Context
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState

/** Harmless local media session exercising the native media path. It holds
 * no audio: the session state machine alone drives playback, metadata, and
 * control callbacks, which is all the native backend observes. */
object TestMediaSession {
    private const val TAG = "HandoverTest"
    private const val DURATION_MS = 180_000L
    private var session: MediaSession? = null
    private var track = 1
    private var position = 0L
    private var playing = false

    fun isActive(): Boolean = session?.isActive == true

    fun start(context: Context) {
        if (session?.isActive == true) return
        track = 1
        position = 0L
        playing = true
        val mediaSession = MediaSession(context, TAG)
        mediaSession.setCallback(object : MediaSession.Callback() {
            override fun onPlay() = applyState(playing = true)
            override fun onPause() = applyState(playing = false)
            override fun onSkipToNext() {
                track += 1
                applyState(playing = true)
            }

            override fun onSkipToPrevious() {
                if (track > 1) track -= 1
                applyState(playing = true)
            }

            override fun onSeekTo(pos: Long) {
                position = pos.coerceIn(0, DURATION_MS)
                publish(mediaSession)
            }
        })
        // Every advertised control has a genuine callback behind it.
        mediaSession.setPlaybackState(
            PlaybackState.Builder()
                .setActions(
                    PlaybackState.ACTION_PLAY
                        or PlaybackState.ACTION_PAUSE
                        or PlaybackState.ACTION_PLAY_PAUSE
                        or PlaybackState.ACTION_SKIP_TO_NEXT
                        or PlaybackState.ACTION_SKIP_TO_PREVIOUS
                        or PlaybackState.ACTION_SEEK_TO,
                )
                .setState(PlaybackState.STATE_PLAYING, position, 1.0f)
                .build(),
        )
        mediaSession.setMetadata(
            MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, "Test track $track")
                .putString(MediaMetadata.METADATA_KEY_ARTIST, "Handover")
                .putString(MediaMetadata.METADATA_KEY_ALBUM, "Live verification")
                .putLong(MediaMetadata.METADATA_KEY_DURATION, DURATION_MS)
                .build(),
        )
        mediaSession.isActive = true
        session = mediaSession
    }

    fun stop() {
        session?.run {
            isActive = false
            release()
        }
        session = null
    }

    private fun applyState(playing: Boolean) {
        this.playing = playing
        session?.let(::publish)
    }

    private fun publish(mediaSession: MediaSession) {
        mediaSession.setMetadata(
            MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, "Test track $track")
                .putString(MediaMetadata.METADATA_KEY_ARTIST, "Handover")
                .putString(MediaMetadata.METADATA_KEY_ALBUM, "Live verification")
                .putLong(MediaMetadata.METADATA_KEY_DURATION, DURATION_MS)
                .build(),
        )
        mediaSession.setPlaybackState(
            PlaybackState.Builder()
                .setActions(
                    PlaybackState.ACTION_PLAY
                        or PlaybackState.ACTION_PAUSE
                        or PlaybackState.ACTION_PLAY_PAUSE
                        or PlaybackState.ACTION_SKIP_TO_NEXT
                        or PlaybackState.ACTION_SKIP_TO_PREVIOUS
                        or PlaybackState.ACTION_SEEK_TO,
                )
                .setState(
                    if (playing) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED,
                    position,
                    1.0f,
                )
                .build(),
        )
    }
}
