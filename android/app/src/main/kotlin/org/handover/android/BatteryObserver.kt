package org.handover.android

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.BatteryManager

data class BatteryReading(val percentage: Int, val charging: Boolean)

/** Event driven battery source. Registering also returns the current sticky battery intent. */
class BatteryObserver(private val context: Context, private val onReading: (BatteryReading) -> Unit) {
    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            reading(intent)?.let(onReading)
        }
    }

    fun start() {
        val initial = context.registerReceiver(receiver, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        initial?.let { reading(it)?.let(onReading) }
    }

    fun stop() = context.unregisterReceiver(receiver)

    companion object {
        fun reading(intent: Intent): BatteryReading? {
            val level = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
            val scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, -1)
            val status = intent.getIntExtra(BatteryManager.EXTRA_STATUS, -1)
            return reading(level, scale, status)
        }

        fun reading(level: Int, scale: Int, status: Int): BatteryReading? {
            if (level < 0 || scale <= 0) return null
            val charging = status == BatteryManager.BATTERY_STATUS_CHARGING ||
                status == BatteryManager.BATTERY_STATUS_FULL
            return BatteryReading((level * 100 / scale).coerceIn(0, 100), charging)
        }
    }
}
