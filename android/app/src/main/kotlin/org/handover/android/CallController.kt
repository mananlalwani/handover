package org.handover.android

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.telecom.TelecomManager
import android.telephony.PhoneNumberUtils
import android.telephony.TelephonyManager

/** Permission and observed-state gated controls. True means Android accepted
 * the request, never that a call connected. No automatic retries. */
object CallController {
    fun hasPermission(context: Context, permission: String): Boolean =
        context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    @Suppress("DEPRECATION")
    private fun state(context: Context): Int? = runCatching {
        if (!hasPermission(context, Manifest.permission.READ_PHONE_STATE)) return null
        context.getSystemService(TelephonyManager::class.java).callState
    }.getOrNull()

    @Suppress("DEPRECATION")
    fun place(context: Context, address: String): Boolean {
        if (!validCallAddress(address) || !hasPermission(context, Manifest.permission.CALL_PHONE)
            || state(context) != TelephonyManager.CALL_STATE_IDLE) return false
        return runCatching {
            val emergency = if (Build.VERSION.SDK_INT >= 29) {
                context.getSystemService(TelephonyManager::class.java).isEmergencyNumber(address)
            } else PhoneNumberUtils.isEmergencyNumber(address)
            if (emergency) return false
            context.getSystemService(TelecomManager::class.java)
                .placeCall(Uri.fromParts("tel", address, null), Bundle())
            true
        }.getOrDefault(false)
    }

    @Suppress("DEPRECATION")
    fun answer(context: Context): Boolean {
        if (!hasPermission(context, Manifest.permission.ANSWER_PHONE_CALLS)
            || state(context) != TelephonyManager.CALL_STATE_RINGING) return false
        return runCatching {
            context.getSystemService(TelecomManager::class.java).acceptRingingCall()
            true
        }.getOrDefault(false)
    }

    @Suppress("DEPRECATION")
    fun hangup(context: Context, decline: Boolean = false): Boolean {
        val expected = if (decline) TelephonyManager.CALL_STATE_RINGING else TelephonyManager.CALL_STATE_OFFHOOK
        if (!hasPermission(context, Manifest.permission.ANSWER_PHONE_CALLS)
            || state(context) != expected) return false
        return runCatching { context.getSystemService(TelecomManager::class.java).endCall() }.getOrDefault(false)
    }
}
