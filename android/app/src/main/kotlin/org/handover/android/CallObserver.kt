package org.handover.android

import android.Manifest
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.telephony.PhoneStateListener
import android.telephony.SubscriptionManager
import android.telephony.TelephonyManager
import org.json.JSONArray
import org.json.JSONObject

/** Aggregates cellular call state across active subscriptions. It falls back
 * to the default TelephonyManager when subscription enumeration is unavailable.
 * No subscription identifiers, phone numbers, or call logs leave this class. */
@Suppress("DEPRECATION")
class CallObserver(private val context: Context, private val publish: (JSONObject) -> Unit) {
    private val handler = Handler(Looper.getMainLooper())
    private val baseTelephony = context.getSystemService(TelephonyManager::class.java)
    private val subscriptions = context.getSystemService(SubscriptionManager::class.java)
    private val listeners = mutableListOf<Pair<TelephonyManager, PhoneStateListener>>()
    private val states = mutableMapOf<Int, Int>()
    private var stopped = false
    private val subscriptionListener = object : SubscriptionManager.OnSubscriptionsChangedListener() {
        override fun onSubscriptionsChanged() { if (!stopped) registerListeners() }
    }

    @Volatile var generation: Long = -1
        private set

    fun refresh() = handler.post {
        if (stopped) return@post
        if (!CallController.hasPermission(context, Manifest.permission.READ_PHONE_STATE)) {
            unregisterListeners(); emit(null); return@post
        }
        registerListeners()
        emit(aggregateState())
    }

    private fun registerListeners() {
        unregisterListeners()
        val managers = runCatching {
            subscriptions.activeSubscriptionInfoList.orEmpty()
                .map { baseTelephony.createForSubscriptionId(it.subscriptionId) }
        }.getOrDefault(emptyList()).ifEmpty { listOf(baseTelephony) }
        managers.forEachIndexed { index, manager ->
            val listener = object : PhoneStateListener() {
                override fun onCallStateChanged(state: Int, ignoredNumber: String?) {
                    states[index] = state
                    if (!stopped) emit(aggregateState())
                }
            }
            runCatching { manager.listen(listener, PhoneStateListener.LISTEN_CALL_STATE) }
                .onSuccess { listeners += manager to listener }
        }
        runCatching { subscriptions.addOnSubscriptionsChangedListener(subscriptionListener) }
    }

    private fun aggregateState(): Int? {
        val values = states.values
        return when {
            values.any { it == TelephonyManager.CALL_STATE_RINGING } -> TelephonyManager.CALL_STATE_RINGING
            values.any { it == TelephonyManager.CALL_STATE_OFFHOOK } -> TelephonyManager.CALL_STATE_OFFHOOK
            values.any { it == TelephonyManager.CALL_STATE_IDLE } -> TelephonyManager.CALL_STATE_IDLE
            else -> runCatching { baseTelephony.callState }.getOrNull()
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
        if (phase == "idle" && CallController.hasPermission(context, Manifest.permission.CALL_PHONE)) controls.put("place")
        if (CallController.hasPermission(context, Manifest.permission.ANSWER_PHONE_CALLS)) {
            if (phase == "ringing") { controls.put("answer"); controls.put("decline") }
            if (phase == "off_hook") controls.put("hangup")
        }
        publish(JSONObject().put("type", "call_state").put("protocol", 1)
            .put("phase", phase).put("controls", controls).put("generation", ++generation))
    }

    private fun unregisterListeners() {
        listeners.forEach { (manager, listener) -> runCatching { manager.listen(listener, PhoneStateListener.LISTEN_NONE) } }
        listeners.clear(); states.clear()
    }

    fun stop() = handler.post {
        stopped = true
        unregisterListeners()
        runCatching { subscriptions.removeOnSubscriptionsChangedListener(subscriptionListener) }
    }
}
