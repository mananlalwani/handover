package org.handover.android

import android.content.Intent
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.view.View
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView

internal fun MainActivity.dp(value: Int) = (value * resources.displayMetrics.density).toInt()

internal fun MainActivity.panel(vararg children: View) = LinearLayout(this).apply {
    orientation = LinearLayout.VERTICAL
    setPadding(dp(18), dp(16), dp(18), dp(16))
    background = GradientDrawable().apply {
        setColor(Color.rgb(247, 248, 252))
        cornerRadius = dp(18).toFloat()
        setStroke(dp(1), Color.rgb(224, 227, 235))
    }
    children.forEach { addView(it, LinearLayout.LayoutParams(-1, -2)) }
}

internal fun MainActivity.sectionTitle(text: String) = TextView(this).apply {
    this.text = text
    textSize = 18f
    setTextColor(Color.rgb(28, 34, 48))
    setTypeface(typeface, Typeface.BOLD)
    setPadding(0, 0, 0, dp(10))
}

internal fun MainActivity.primary(button: Button) = button.apply {
    isAllCaps = false
    textSize = 15f
    setTextColor(Color.WHITE)
    backgroundTintList = android.content.res.ColorStateList.valueOf(Color.rgb(48, 84, 210))
}

internal fun MainActivity.secondary(button: Button) = button.apply {
    isAllCaps = false
    textSize = 14f
}

internal fun MainActivity.refreshPermissionButtons() {
    permissionButtons.forEach { (button, granted) ->
        val enabled = granted()
        button.backgroundTintList = android.content.res.ColorStateList.valueOf(
            if (enabled) Color.rgb(35, 142, 84) else Color.rgb(105, 70, 190),
        )
        button.setTextColor(Color.WHITE)
        button.alpha = if (enabled) 0.88f else 1f
    }
}

internal fun MainActivity.startHandoverConnection() {
    val service = Intent(this, HandoverForegroundService::class.java)
    startForegroundService(service)
    AppUpdater.clearReconnectNeeded(this)
    refreshStatus()
}

internal fun MainActivity.menuButton(title: String, description: String, action: () -> Unit): LinearLayout {
    val activity = this
    return LinearLayout(activity).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(activity.dp(18), activity.dp(16), activity.dp(18), activity.dp(16))
        background = GradientDrawable().apply {
            setColor(Color.WHITE)
            cornerRadius = activity.dp(18).toFloat()
            setStroke(activity.dp(1), Color.rgb(224, 227, 235))
        }
        isClickable = true
        isFocusable = true
        foreground = activity.getDrawable(android.R.drawable.list_selector_background)
        addView(TextView(activity).apply {
            text = title
            textSize = 17f
            setTextColor(Color.rgb(28, 34, 48))
            setTypeface(typeface, Typeface.BOLD)
        })
        addView(TextView(activity).apply {
            text = description
            textSize = 13f
            setTextColor(Color.rgb(90, 98, 114))
        }, LinearLayout.LayoutParams(-1, -2).apply { topMargin = activity.dp(4) })
        setOnClickListener { action() }
    }
}

internal fun MainActivity.showPage(page: View) {
    pageHost.removeAllViews()
    (page.parent as? android.view.ViewGroup)?.removeView(page)
    pageHost.addView(page, LinearLayout.LayoutParams(-1, -2))
}
