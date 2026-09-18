package org.handover.android

import android.annotation.SuppressLint
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
    data class Result(val accepted: Boolean, val failure: String? = null)
    private fun rejected(reason: String) = Result(false, reason)
    fun hasPermission(context: Context, permission: String): Boolean =
        context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    @Suppress("DEPRECATION")
    private fun state(context: Context): Int? = runCatching {
        if (!hasPermission(context, Manifest.permission.READ_PHONE_STATE)) return null
        context.getSystemService(TelephonyManager::class.java).callState
    }.getOrNull()

    @Suppress("DEPRECATION")
    @SuppressLint("MissingPermission")
    fun place(context: Context, address: String): Result {
        if (!validCallAddress(address)) return rejected("invalid_address")
        if (!hasPermission(context, Manifest.permission.CALL_PHONE)) return rejected("permission_denied")
        if (state(context) != TelephonyManager.CALL_STATE_IDLE) return rejected("wrong_phase")
        return runCatching {
            val emergency = if (Build.VERSION.SDK_INT >= 29) {
                context.getSystemService(TelephonyManager::class.java).isEmergencyNumber(address)
            } else PhoneNumberUtils.isEmergencyNumber(address)
            if (emergency) return rejected("emergency_number")
            context.getSystemService(TelecomManager::class.java)
                .placeCall(Uri.fromParts("tel", address, null), Bundle())
            Result(true)
        }.getOrElse { rejected("rejected") }
    }

    @Suppress("DEPRECATION")
    @SuppressLint("MissingPermission")
    fun answer(context: Context): Result {
        if (!hasPermission(context, Manifest.permission.ANSWER_PHONE_CALLS)) return rejected("permission_denied")
        if (state(context) != TelephonyManager.CALL_STATE_RINGING) return rejected("wrong_phase")
        return runCatching {
            context.getSystemService(TelecomManager::class.java).acceptRingingCall()
            Result(true)
        }.getOrElse { rejected("rejected") }
    }

    @Suppress("DEPRECATION")
    @SuppressLint("MissingPermission", "NewApi")
    fun hangup(context: Context, decline: Boolean = false): Result {
        val expected = if (decline) TelephonyManager.CALL_STATE_RINGING else TelephonyManager.CALL_STATE_OFFHOOK
        if (!hasPermission(context, Manifest.permission.ANSWER_PHONE_CALLS)) return rejected("permission_denied")
        if (state(context) != expected) return rejected("wrong_phase")
        return runCatching {
            if (Build.VERSION.SDK_INT < 28) rejected("unsupported")
            else if (context.getSystemService(TelecomManager::class.java).endCall()) Result(true)
            else rejected("rejected")
        }.getOrElse { rejected("rejected") }
    }
}
