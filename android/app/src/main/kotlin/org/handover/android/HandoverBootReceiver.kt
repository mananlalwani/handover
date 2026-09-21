package org.handover.android

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Restarts the native connection after boot when a desktop is already paired. */
class HandoverBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            Intent.ACTION_BOOT_COMPLETED,
            Intent.ACTION_LOCKED_BOOT_COMPLETED,
            "android.intent.action.QUICKBOOT_POWERON",
            "com.htc.intent.action.QUICKBOOT_POWERON" ->
                HandoverForegroundService.startIfPaired(context)
        }
    }
}
