package org.handover.android

import android.media.AudioManager
import android.os.Build
import org.json.JSONArray
import org.json.JSONObject

internal data class PhoneVolumeRequest(val requestId: String, val kind: String, val level: Int)

internal fun parsePhoneVolumeRequest(message: JSONObject): PhoneVolumeRequest? {
    val requestId = message.opt("request_id") as? String ?: return null
    if (!requestId.matches(Regex("[0-9a-f]{32}"))) return null
    val kind = message.opt("kind") as? String ?: return null
    if (kind !in PHONE_VOLUME_STREAMS.map { it.kind }) return null
    val rawLevel = message.opt("level")
    val level = when (rawLevel) {
        is Int -> rawLevel
        is Long -> if (rawLevel in Int.MIN_VALUE..Int.MAX_VALUE) rawLevel.toInt() else return null
        else -> return null
    }
    return PhoneVolumeRequest(requestId, kind, level)
}

private data class PhoneVolumeStream(val kind: String, val stream: Int)

private val PHONE_VOLUME_STREAMS = listOf(
    PhoneVolumeStream("media", AudioManager.STREAM_MUSIC),
    PhoneVolumeStream("ringtone", AudioManager.STREAM_RING),
    PhoneVolumeStream("alarm", AudioManager.STREAM_ALARM),
    PhoneVolumeStream("call", AudioManager.STREAM_VOICE_CALL),
)

internal fun NativeTransport.phoneVolumeState(): JSONObject {
    val manager = runCatching { context.getSystemService(AudioManager::class.java) }.getOrNull()
    val streams = JSONArray()
    PHONE_VOLUME_STREAMS.forEach { item ->
        if (manager == null) return@forEach
        runCatching {
            val min = if (Build.VERSION.SDK_INT >= 28) manager.getStreamMinVolume(item.stream) else 0
            val max = manager.getStreamMaxVolume(item.stream)
            val level = manager.getStreamVolume(item.stream)
            require(min >= 0 && min <= level && level <= max)
            streams.put(JSONObject().put("kind", item.kind).put("min_level", min).put("max_level", max)
                .put("level", level).put("muted", manager.isStreamMute(item.stream))
                .put("adjustable", !manager.isVolumeFixed && max > min))
        }
    }
    return JSONObject().put("type", "volume_state").put("protocol", 1).put("streams", streams)
}

internal fun NativeTransport.sendPhoneVolumeState() {
    val capturedSocket = socket ?: return
    val capturedOutput = output ?: return
    val state = phoneVolumeState()
    enqueue { synchronized(outputLock) {
        if (socket === capturedSocket && output === capturedOutput) write(capturedOutput,state)
    } }
}

internal fun NativeTransport.handlePhoneVolumeSet(message: JSONObject) {
    val capturedSocket = socket ?: return
    val capturedOutput = output ?: return
    fun reply(value: JSONObject) { enqueue { synchronized(outputLock) {
        if (socket === capturedSocket && output === capturedOutput) write(capturedOutput,value)
    } } }
    val request = parsePhoneVolumeRequest(message)
    if (request == null) {
        reply(JSONObject().put("type", "volume_result").put("protocol", 1)
            .put("request_id", message.optString("request_id")).put("accepted", false).put("failure", "invalid_value"))
        reply(phoneVolumeState())
        return
    }

    var failure: String? = null
    try {
        val manager = context.getSystemService(AudioManager::class.java)
        if (manager == null) {
            failure = "unavailable"
        } else {
            val stream = PHONE_VOLUME_STREAMS.first { it.kind == request.kind }.stream
            val min = if (Build.VERSION.SDK_INT >= 28) manager.getStreamMinVolume(stream) else 0
            val max = manager.getStreamMaxVolume(stream)
            if (request.level !in min..max || (Build.VERSION.SDK_INT >= 21 && manager.isVolumeFixed)) {
                failure = "invalid_value"
            } else {
                manager.setStreamVolume(stream, request.level, 0)
            }
        }
    } catch (_: SecurityException) {
        failure = "permission_required"
    } catch (_: RuntimeException) {
        failure = "unavailable"
    }

    reply(JSONObject().put("type", "volume_result").put("protocol", 1)
        .put("request_id", request.requestId).put("accepted", failure == null).apply {
            failure?.let { put("failure", it) }
        })
    reply(phoneVolumeState())
}
