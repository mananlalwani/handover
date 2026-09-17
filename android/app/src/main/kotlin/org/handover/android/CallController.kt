package org.handover.android

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.telecom.TelecomManager
import android.telephony.PhoneNumberUtils

/** Explicit cellular-call controls. Every operation is permission-gated and
 * reports only whether Android accepted the request, never call success. */
object CallController {
    fun place(context: Context, address: String): Boolean {
        val number = address.trim()
        if (number.isEmpty() || number.length > 64 || PhoneNumberUtils.isEmergencyNumber(number)) return false
        if (context.checkSelfPermission(Manifest.permission.CALL_PHONE) != PackageManager.PERMISSION_GRANTED) return false
        return runCatching {
            context.startActivity(Intent(Intent.ACTION_CALL, Uri.fromParts("tel", number, null))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            true
        }.getOrDefault(false)
    }

    @Suppress("DEPRECATION")
    fun answer(context: Context): Boolean {
        if (context.checkSelfPermission(Manifest.permission.ANSWER_PHONE_CALLS) != PackageManager.PERMISSION_GRANTED) return false
        return runCatching {
            context.getSystemService(TelecomManager::class.java).acceptRingingCall()
            true
        }.getOrDefault(false)
    }

    @Suppress("DEPRECATION")
    fun hangup(context: Context): Boolean {
        if (context.checkSelfPermission(Manifest.permission.ANSWER_PHONE_CALLS) != PackageManager.PERMISSION_GRANTED) return false
        return runCatching { context.getSystemService(TelecomManager::class.java).endCall() }.getOrDefault(false)
    }
}
