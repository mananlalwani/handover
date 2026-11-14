package org.handover.android

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LaptopAudioDiagnosticsTest {
    private fun stat(type: String, vararg values: Pair<String, Any>) =
        LaptopAudioDiagnostics.Stat(type, mapOf(*values))

    @Test fun missingMetricsStayAbsent() {
        val result = LaptopAudioDiagnostics().sample(
            mapOf("in" to stat("inbound-rtp", "kind" to "audio")),
            timestampUs = 1_000_000.0,
            vpnAddresses = emptySet(),
        )

        assertEquals("unknown", result.getString("route"))
        listOf(
            "transport", "round_trip_ms", "jitter_ms", "jitter_buffer_ms",
            "bitrate_bps", "packets_received", "packets_lost",
        ).forEach { assertFalse("$it should be absent", result.has(it)) }
    }

    @Test fun onlyActuallySelectedCandidateDeterminesRoute() {
        val stats = mapOf(
            "pair-nominated" to stat(
                "candidate-pair", "nominated" to true,
                "localCandidateId" to "vpn", "remoteCandidateId" to "remote",
            ),
            "vpn" to stat("local-candidate", "candidateType" to "host", "address" to "10.0.0.2"),
            "remote" to stat("remote-candidate", "candidateType" to "srflx"),
        )

        val nominatedOnly = LaptopAudioDiagnostics().sample(stats, 1_000_000.0, setOf("10.0.0.2"))
        assertEquals("unknown", nominatedOnly.getString("route"))

        val selectedStats = stats + ("pair-nominated" to stat(
            "candidate-pair", "selected" to true, "nominated" to true,
            "localCandidateId" to "vpn", "remoteCandidateId" to "remote",
        ))
        val selected = LaptopAudioDiagnostics().sample(selectedStats, 1_000_000.0, setOf("10.0.0.2"))
        assertEquals("vpn", selected.getString("route"))
    }

    @Test fun relayCandidateTakesPrecedenceOverVpnAddress() {
        val result = LaptopAudioDiagnostics().sample(
            mapOf(
                "pair" to stat("candidate-pair", "selected" to true, "localCandidateId" to "local", "remoteCandidateId" to "remote"),
                "local" to stat("local-candidate", "candidateType" to "host", "address" to "10.0.0.2"),
                "remote" to stat("remote-candidate", "candidateType" to "relay"),
            ),
            1_000_000.0,
            setOf("10.0.0.2"),
        )
        assertEquals("relay", result.getString("route"))
    }

    @Test fun rttJitterAndLifetimeAverageBufferAreConvertedToMilliseconds() {
        val result = LaptopAudioDiagnostics().sample(
            mapOf(
                "in" to stat(
                    "inbound-rtp", "kind" to "audio", "transportId" to "transport",
                    "jitter" to 0.0129, "jitterBufferDelay" to 0.48,
                    "jitterBufferEmittedCount" to 40,
                ),
                "transport" to stat("transport", "selectedCandidatePairId" to "pair"),
                "pair" to stat("candidate-pair", "currentRoundTripTime" to 0.0349),
            ),
            1_000_000.0,
            emptySet(),
        )
        assertEquals(34, result.getInt("round_trip_ms"))
        assertEquals(12, result.getInt("jitter_ms"))
        assertEquals(12, result.getInt("jitter_buffer_ms"))
    }

    @Test fun bitrateNeedsTwoSamplesAndCounterResetStartsANewBaseline() {
        val diagnostics = LaptopAudioDiagnostics()
        fun sample(bytes: Long, timestampUs: Double) = diagnostics.sample(
            mapOf("in" to stat("inbound-rtp", "kind" to "audio", "bytesReceived" to bytes)),
            timestampUs,
            emptySet(),
        )

        assertFalse(sample(1_000, 1_000_000.0).has("bitrate_bps"))
        assertEquals(8_000, sample(2_000, 2_000_000.0).getInt("bitrate_bps"))
        assertFalse(sample(100, 3_000_000.0).has("bitrate_bps"))
        assertEquals(8_000, sample(1_100, 4_000_000.0).getInt("bitrate_bps"))
    }
}
