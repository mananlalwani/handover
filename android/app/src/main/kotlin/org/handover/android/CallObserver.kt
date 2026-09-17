package org.handover.android

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Handler
import android.os.Looper
import android.telephony.PhoneStateListener
import android.telephony.TelephonyManager
import org.json.JSONArray
import org.json.JSONObject

/** Aggregate cellular state only: OFFHOOK includes dialing, not confirmed answer.
 * No phone numbers or call logs are collected. All listener lifecycle runs on main. */
@Suppress("DEPRECATION")
class CallObserver(private val context: Context, private val publish: (JSONObject) -> Unit) {
    private val handler = Handler(Looper.getMainLooper())
    private val telephony = context.getSystemService(TelephonyManager::class.java)
    private var listener: PhoneStateListener? = null
    private var stopped = false

    /** Latest published call-state generation. Command execution re-checks it
     * so a queued command can never act on re-observed (later) state. */
    @Volatile var generation: Long = -1
        private set

    fun refresh() {
        handler.post {
            if (stopped) return@post
            if (!CallController.hasPermission(context, Manifest.permission.READ_PHONE_STATE)) {
                unregister()
                generation = -1
                emit(null)
                return@post
            }
            try {
                if (listener == null) {
                    val next = object : PhoneStateListener() {
                        override fun onCallStateChanged(state: Int, ignoredNumber: String?) {
                            if (!stopped) emit(state)
                        }
                    }
                    telephony.listen(next, PhoneStateListener.LISTEN_CALL_STATE)
                    listener = next
                }
                emit(telephony.callState)
            } catch (_: Exception) {
                unregister()
                emit(null)
            }
        }
    }

    private fun emit(state: Int?) {
        val phase = when (state) {
            TelephonyManager.CALL_STATE_IDLE -> "idle"
            TelephonyManager.CALL_STATE_RINGING -> "ringing"
            TelephonyManager.CALL_STATE_OFFHOOK -> "off_hook"
            else -> "unknown"
        }
        val controls = JSONArray()
        if (phase == "idle" && CallController.hasPermission(context, Manifest.permission.CALL_PHONE)) {
            controls.put("place")
        }
        if (CallController.hasPermission(context, Manifest.permission.ANSWER_PHONE_CALLS)) {
            if (phase == "ringing") { controls.put("answer"); controls.put("decline") }
            if (phase == "off_hook") controls.put("hangup")
        }
        publish(JSONObject().put("type", "call_state").put("protocol", 1)
            .put("phase", phase).put("controls", controls).put("generation", ++generation))
    }

    private fun unregister() {
        listener?.let { runCatching { telephony.listen(it, PhoneStateListener.LISTEN_NONE) } }
        listener = null
    }

    fun stop() {
        handler.post { stopped = true; unregister() }
    }
}
