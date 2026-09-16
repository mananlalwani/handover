package org.handover.android

import android.content.Context
import java.util.UUID

/** Stable installation identity. The private preferences file is never logged or sent by this shell. */
class DeviceIdentityStore(context: Context) {
    private val preferences = context.getSharedPreferences("handover_identity", Context.MODE_PRIVATE)

    val deviceId: String
        get() = preferences.getString(KEY_DEVICE_ID, null) ?: UUID.randomUUID().toString().also {
            preferences.edit().putString(KEY_DEVICE_ID, it).apply()
        }

    companion object {
        private const val KEY_DEVICE_ID = "device_id"
    }
}
