package org.handover.android

import org.json.JSONObject

/** Reads observations only. Missing statistics stay absent, not zero estimates. */
internal class LaptopAudioDiagnostics {
    data class Stat(val type: String, val members: Map<String, Any>)
    private var previousBytes: Long? = null
    private var previousTimeUs: Double? = null

    fun sample(stats: Map<String, Stat>, timestampUs: Double, vpnAddresses: Set<String>): JSONObject {
        val inbound = stats.values.firstOrNull {
            it.type == "inbound-rtp" && (it.members["kind"] == "audio" || it.members["mediaType"] == "audio")
        }?.members
        val transport = (inbound?.get("transportId") as? String)?.let { stats[it]?.members }
            ?: stats.values.firstOrNull { it.type == "transport" && it.members["selectedCandidatePairId"] is String }?.members
        val pair = (transport?.get("selectedCandidatePairId") as? String)?.let { stats[it]?.members }
            ?: stats.values.firstOrNull { it.type == "candidate-pair" && it.members["selected"] == true }?.members
        val local = (pair?.get("localCandidateId") as? String)?.let { stats[it]?.members }
        val remote = (pair?.get("remoteCandidateId") as? String)?.let { stats[it]?.members }
        val address = (local?.get("address") ?: local?.get("ip")) as? String
        val route = when {
            local?.get("candidateType") == "relay" || remote?.get("candidateType") == "relay" -> "relay"
            address != null && vpnAddresses.contains(address.trim('[', ']').lowercase()) -> "vpn"
            pair != null && local?.get("candidateType") in listOf("host", "srflx", "prflx") -> "direct"
            else -> "unknown"
        }
        fun number(values: Map<String, Any>?, key: String): Double? =
            (values?.get(key) as? Number)?.toDouble()?.takeIf { it.isFinite() && it >= 0 }
        fun millis(value: Double?): Long? = value?.times(1000)?.takeIf { it <= 60_000 }?.toLong()
        val bytes = number(inbound, "bytesReceived")?.toLong()
        val elapsed = previousTimeUs?.let { timestampUs - it }
        val rate = if (bytes != null && previousBytes != null && elapsed != null && elapsed > 0 && bytes >= previousBytes!!) {
            ((bytes - previousBytes!!) * 8.0 * 1_000_000 / elapsed).takeIf { it <= 10_000_000 }?.toLong()
        } else null
        previousBytes = bytes; previousTimeUs = timestampUs
        val delay = number(inbound, "jitterBufferDelay")
        val emitted = number(inbound, "jitterBufferEmittedCount")
        return JSONObject().put("route", route).apply {
            (local?.get("protocol") as? String)?.lowercase()?.takeIf { it == "udp" || it == "tcp" }?.let { put("transport", it) }
            millis(number(pair, "currentRoundTripTime"))?.let { put("round_trip_ms", it) }
            millis(number(inbound, "jitter"))?.let { put("jitter_ms", it) }
            if (delay != null && emitted != null && emitted > 0) millis(delay / emitted)?.let { put("jitter_buffer_ms", it) }
            rate?.let { put("bitrate_bps", it) }
            number(inbound, "packetsReceived")?.let { put("packets_received", it.toLong()) }
            number(inbound, "packetsLost")?.let { put("packets_lost", it.toLong()) }
        }
    }
}
