package org.handover.android

import android.content.Intent
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.LinearLayout
import android.widget.Switch
import android.widget.TextView

internal fun MainActivity.dp(value: Int) = (value * resources.displayMetrics.density).toInt()

internal fun MainActivity.cardBackground() = GradientDrawable().apply {
    setColor(Color.WHITE)
    cornerRadius = dp(18).toFloat()
    setStroke(dp(1), Color.rgb(226, 229, 236))
}

internal fun MainActivity.panel(vararg children: View) = LinearLayout(this).apply {
    orientation = LinearLayout.VERTICAL
    setPadding(dp(16), dp(16), dp(16), dp(16))
    background = cardBackground()
    children.forEachIndexed { index, child ->
        if (child is Switch) {
            child.minHeight = dp(48)
            child.setTextColor(Color.rgb(28, 34, 48))
            child.textSize = 15f
        }
        addView(child, LinearLayout.LayoutParams(-1, -2).apply {
            if (index > 0) topMargin = dp(8)
        })
    }
}

internal fun MainActivity.sectionTitle(text: String) = TextView(this).apply {
    this.text = text
    textSize = 17f
    setTextColor(Color.rgb(28, 34, 48))
    setTypeface(typeface, Typeface.BOLD)
    androidx.core.view.ViewCompat.setAccessibilityHeading(this, true)
}

private fun MainActivity.actionButton(button: Button, prominent: Boolean) = button.apply {
    isAllCaps = false
    textSize = 15f
    setTypeface(Typeface.create("sans-serif-medium", Typeface.NORMAL))
    gravity = Gravity.CENTER
    minHeight = dp(48)
    minimumHeight = dp(48)
    minWidth = 0
    minimumWidth = 0
    setPadding(dp(12), dp(10), dp(12), dp(10))
    stateListAnimator = null
    elevation = 0f
    backgroundTintList = null
    background = RippleDrawable(ColorStateList.valueOf(Color.argb(32, 48, 84, 210)), GradientDrawable().apply {
        setColor(if (prominent) Color.rgb(48, 84, 210) else Color.rgb(238, 242, 253))
        cornerRadius = dp(12).toFloat()
    }, null)
    setTextColor(ColorStateList(
        arrayOf(intArrayOf(-android.R.attr.state_enabled), intArrayOf()),
        intArrayOf(Color.rgb(130, 139, 159), if (prominent) Color.WHITE else Color.rgb(42, 72, 170)),
    ))
}

internal fun MainActivity.primary(button: Button) = actionButton(button, true)

internal fun MainActivity.secondary(button: Button) = actionButton(button, false)

internal fun MainActivity.refreshPermissionButtons() {
    permissionButtons.forEach { (button, granted) ->
        val enabled = granted()
        val label = (button.tag as? String) ?: button.text.toString().also { button.tag = it }
        val (name, allowed, denied) = when (label) {
            "Enable notification access" -> Triple("Notification access", "Allowed", "Off")
            "Enable app notifications" -> Triple("App notifications", "Allowed", "Off")
            "Enable call controls" -> Triple("Call controls", "Allowed", "Off")
            "Enable remote lock" -> Triple("Remote lock", "Allowed", "Off")
            "Enable local network" -> Triple("Local network", "Available", "Off")
            "Review battery optimization" -> Triple("Battery usage", "Unrestricted", "Optimized")
            else -> Triple(label, "Allowed", "Off")
        }
        button.text = "$name: ${if (enabled) allowed else denied}"
        button.backgroundTintList = ColorStateList.valueOf(
            if (enabled) Color.rgb(232, 245, 237) else Color.rgb(238, 242, 253),
        )
        button.setTextColor(if (enabled) Color.rgb(32, 105, 62) else Color.rgb(42, 72, 170))
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
        setPadding(activity.dp(16), activity.dp(14), activity.dp(16), activity.dp(14))
        minimumHeight = activity.dp(64)
        isClickable = true
        isFocusable = true
        foreground = RippleDrawable(ColorStateList.valueOf(Color.argb(24, 48, 84, 210)), null, null)
        addView(TextView(activity).apply {
            text = title
            textSize = 16f
            setTextColor(Color.rgb(28, 34, 48))
            setTypeface(typeface, Typeface.BOLD)
            setCompoundDrawablesRelativeWithIntrinsicBounds(null, null, activity.getDrawable(R.drawable.ic_chevron_right), null)
        })
        addView(TextView(activity).apply {
            text = description
            textSize = 13f
            setTextColor(Color.rgb(90, 98, 114))
        }, LinearLayout.LayoutParams(-1, -2).apply { topMargin = activity.dp(3) })
        setOnClickListener { action() }
    }
}

internal fun MainActivity.navigationGroup(title: String, vararg rows: View) = LinearLayout(this).apply {
    orientation = LinearLayout.VERTICAL
    addView(sectionTitle(title).apply {
        textSize = 14f
        setTextColor(Color.rgb(92, 99, 116))
        setPadding(dp(4), dp(20), 0, dp(8))
    })
    addView(LinearLayout(this@navigationGroup).apply {
        orientation = LinearLayout.VERTICAL
        background = cardBackground()
        clipToOutline = true
        rows.forEachIndexed { index, row ->
            if (index > 0) addView(View(this@navigationGroup).apply {
                setBackgroundColor(Color.rgb(234, 236, 242))
            }, LinearLayout.LayoutParams(-1, dp(1)).apply { marginStart = dp(16) })
            addView(row, LinearLayout.LayoutParams(-1, -2))
        }
    }, LinearLayout.LayoutParams(-1, -2))
}

internal fun MainActivity.showPage(page: View) {
    val previous = if (pageHost.childCount > 0) pageHost.getChildAt(0) else null
    if (previous === page) return
    pageScroll?.let { scroll -> previous?.let { pagePositions[it] = scroll.scrollY } }
    currentFocus?.let { focused ->
        (getSystemService(android.content.Context.INPUT_METHOD_SERVICE) as android.view.inputmethod.InputMethodManager)
            .hideSoftInputFromWindow(focused.windowToken, 0)
        focused.clearFocus()
    }
    pageHost.removeAllViews()
    (page.parent as? android.view.ViewGroup)?.removeView(page)
    pageHost.addView(page, LinearLayout.LayoutParams(-1, -2))
    pageHost.requestFocus()
    pageScroll?.let { scroll ->
        scroll.addOnLayoutChangeListener(object : View.OnLayoutChangeListener {
            override fun onLayoutChange(view: View, left: Int, top: Int, right: Int, bottom: Int,
                oldLeft: Int, oldTop: Int, oldRight: Int, oldBottom: Int) {
                view.removeOnLayoutChangeListener(this)
                if (pageHost.getChildAt(0) === page) scroll.scrollTo(0, pagePositions[page] ?: 0)
            }
        })
    }
}
