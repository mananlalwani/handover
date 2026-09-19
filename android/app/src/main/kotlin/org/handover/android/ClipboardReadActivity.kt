package org.handover.android

import android.app.Activity
import android.os.Bundle
import android.view.Gravity

/**
 * Briefly gives Handover foreground focus so Android permits one clipboard
 * read. The activity is started only after the opt-in log monitor observes
 * Android denying Handover a background clipboard read.
 */
class ClipboardReadActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.attributes = window.attributes.apply {
            width = 1
            height = 1
            gravity = Gravity.TOP or Gravity.START
            dimAmount = 0f
        }
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus) return
        HandoverForegroundService.sendAutomaticClipboard()
        finish()
    }
}
