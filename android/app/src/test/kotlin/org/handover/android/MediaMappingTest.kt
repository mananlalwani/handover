package org.handover.android

import android.media.session.PlaybackState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MediaMappingTest {
    @Test fun playerValidationMatchesNativeBounds() {
        assertFalse(MediaObserver.isValidPlayer(""))
        assertFalse(MediaObserver.isValidPlayer("p".repeat(129)))
        assertFalse(MediaObserver.isValidPlayer("bad\u0000player"))
        assertTrue(MediaObserver.isValidPlayer("com.example.music"))
    }

    @Test fun playbackMappingNeverInventsPrecision() {
        assertEquals("playing", MediaObserver.playbackOf(PlaybackState.STATE_PLAYING))
        assertEquals("paused", MediaObserver.playbackOf(PlaybackState.STATE_PAUSED))
        assertEquals("stopped", MediaObserver.playbackOf(PlaybackState.STATE_STOPPED))
        // Transitional and error states report unknown rather than guessing.
        assertEquals("unknown", MediaObserver.playbackOf(PlaybackState.STATE_NONE))
        assertEquals("unknown", MediaObserver.playbackOf(PlaybackState.STATE_BUFFERING))
        assertEquals("unknown", MediaObserver.playbackOf(PlaybackState.STATE_ERROR))
        assertEquals("unknown", MediaObserver.playbackOf(PlaybackState.STATE_CONNECTING))
        assertEquals("unknown", MediaObserver.playbackOf(PlaybackState.STATE_FAST_FORWARDING))
        assertEquals("unknown", MediaObserver.playbackOf(PlaybackState.STATE_SKIPPING_TO_NEXT))
    }

    @Test fun controlsMirrorTheActionsBitmask() {
        val full = PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or
            PlaybackState.ACTION_PLAY_PAUSE or PlaybackState.ACTION_SKIP_TO_NEXT or
            PlaybackState.ACTION_SKIP_TO_PREVIOUS or PlaybackState.ACTION_SEEK_TO
        assertEquals(
            listOf("play", "pause", "play_pause", "next", "previous", "set_position"),
            MediaObserver.controlsOf(full),
        )
        assertTrue(MediaObserver.controlsOf(0L).isEmpty())
        // Toggle derives from the discrete bits, matching the KDE adapter.
        assertEquals(
            listOf("play", "pause", "play_pause"),
            MediaObserver.controlsOf(PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE),
        )
        // Relative seeks have no genuine platform API behind them.
        assertFalse("seek" in MediaObserver.controlsOf(full))
    }

    @Test fun postMapUsesVersionedType() {
        val session = WireMediaSession(
            player = "com.example.music",
            application = "Example Music",
            title = "Test track",
            artist = "Test artist",
            album = null,
            playback = "playing",
            positionMs = 1000L,
            durationMs = 180000L,
            controls = listOf("pause", "set_position"),
        )
        val map = MediaObserver.postMap(session)
        assertEquals("media_post", map["type"])
        assertEquals(1, map["protocol"])
        assertEquals("com.example.music", map["player"])
        assertEquals("playing", map["playback"])
        assertEquals(1000L, map["position_ms"])
        @Suppress("UNCHECKED_CAST")
        assertEquals(listOf("pause", "set_position"), map["controls"] as List<String>)
    }

    @Test fun syncMapCarriesTheFullSessionList() {
        val map = MediaObserver.syncMap(
            listOf(
                WireMediaSession("a", "A", null, null, null, "playing", null, null, emptyList()),
                WireMediaSession("b", "B", "t", null, null, "paused", 5L, 10L, listOf("play")),
            ),
        )
        assertEquals("media_sync", map["type"])
        @Suppress("UNCHECKED_CAST")
        val sessions = map["sessions"] as List<Map<String, Any?>>
        assertEquals(2, sessions.size)
        assertEquals("paused", sessions[1]["playback"])
    }

    @Test fun commandValidationRejectsBadPlayersActionsAndPositions() {
        assertFalse(MediaObserver.isValidCommand("dance", null))
        assertFalse(MediaObserver.isValidCommand("seek", 10L))
        assertFalse(MediaObserver.isValidCommand("set_position", null))
        assertFalse(MediaObserver.isValidCommand("set_position", -1L))
        assertFalse(MediaObserver.executeControl("", "pause", null))
        assertFalse(MediaObserver.executeControl("com.example.music", "pause", null))
        assertTrue(MediaObserver.isValidCommand("pause", null))
        assertTrue(MediaObserver.isValidCommand("set_position", 60000L))
    }

    @Test fun controlMapShapeMatchesDaemonExpectations() {
        val map = MediaObserver.controlMap("com.example.music", "set_position", 60000L)
        assertEquals("media_control", map["type"])
        assertEquals(1, map["protocol"])
        assertEquals("set_position", map["action"])
        assertEquals(60000L, map["position_ms"])
    }
}
