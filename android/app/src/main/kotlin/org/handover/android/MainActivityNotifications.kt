package org.handover.android

import android.graphics.Color
import android.graphics.Typeface
import android.text.Editable
import android.text.TextWatcher
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.Switch
import android.widget.TextView

internal fun MainActivity.openNotificationForwarding() {
    val activity = this
    val settings = NotificationForwarding(this)
    val ink = Color.rgb(28, 34, 48)
    val muted = Color.rgb(92, 99, 116)
    val search = EditText(this).apply {
        hint = "Search apps"
        setSingleLine(true)
        textSize = 16f
        setTextColor(ink)
        setHintTextColor(muted)
    }
    val summary = TextView(this).apply {
        text = "Loading apps…"
        textSize = 15f
        setTextColor(ink)
        setTypeface(typeface, Typeface.BOLD)
        setPadding(0, activity.dp(12), 0, activity.dp(4))
        accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
    }
    val matches = TextView(this).apply {
        textSize = 13f
        setTextColor(muted)
        visibility = View.GONE
    }
    val allowAll = secondary(Button(this).apply { text = "Allow all"; isEnabled = false })
    val blockAll = secondary(Button(this).apply { text = "Block all"; isEnabled = false })
    val actions = LinearLayout(this).apply {
        orientation = LinearLayout.HORIZONTAL
        addView(allowAll, LinearLayout.LayoutParams(0, -2, 1f))
        addView(blockAll, LinearLayout.LayoutParams(0, -2, 1f).apply { marginStart = activity.dp(8) })
    }
    val rows = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
    val more = secondary(Button(this).apply { text = "Show more apps"; visibility = View.GONE })
    val page = featurePage(
        "Notifications",
        "Choose which apps send notifications to your laptop. Notifications stay on your phone when you block an app here.",
        panel(TextView(this).apply {
            textSize = 14f
            setTextColor(muted)
            text = if (HandoverNotificationService.isEnabled(activity)) {
                "Notification access is on. Changes are saved automatically."
            } else {
                "Notification access is off. Enable it in Permissions to start forwarding."
            }
        }),
        panel(search, summary, matches, actions, rows, more),
        TextView(this).apply {
            text = "Allow all and Block all apply to every app in this list. New apps are allowed by default.\n\nMissing an app? Let it post a notification, then reopen this page. For chats connected separately on your laptop, use Mute in Messages."
            textSize = 13f
            setTextColor(muted)
        },
    )
    showPage(page)
    var apps = emptyList<NotificationForwarding.App>()
    var limit = 100
    fun updateSummary() {
        val allowed = apps.count { settings.isAllowed(it.packageName) }
        summary.text = "$allowed of ${apps.size} apps allowed"
        allowAll.isEnabled = allowed < apps.size
        blockAll.isEnabled = allowed > 0
    }
    fun render() {
        val query = search.text.toString().trim()
        val filtered = apps.filter {
            it.label.contains(query, ignoreCase = true) || it.packageName.contains(query, ignoreCase = true)
        }
        rows.removeAllViews()
        updateSummary()
        matches.visibility = if (query.isEmpty()) View.GONE else View.VISIBLE
        matches.text = "${filtered.size} matching apps"
        if (filtered.isEmpty()) {
            rows.addView(TextView(activity).apply {
                text = "No matching apps."
                setTextColor(muted)
                setPadding(0, activity.dp(20), 0, activity.dp(20))
            })
        }
        filtered.take(limit).forEachIndexed { index, app ->
            if (index > 0) rows.addView(View(activity).apply {
                setBackgroundColor(Color.rgb(224, 227, 235))
            }, LinearLayout.LayoutParams(-1, activity.dp(1)).apply { marginStart = activity.dp(56) })
            val toggle = Switch(activity).apply {
                tag = app.packageName
                contentDescription = "Forward notifications from ${app.label}"
                isChecked = settings.isAllowed(app.packageName)
                minHeight = activity.dp(48)
                minimumWidth = activity.dp(48)
                setOnCheckedChangeListener { _, allowed ->
                    settings.setAllowed(app.packageName, allowed)
                    updateSummary()
                }
            }
            rows.addView(LinearLayout(activity).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                minimumHeight = activity.dp(76)
                setPadding(0, activity.dp(10), 0, activity.dp(10))
                background = activity.getDrawable(android.R.drawable.list_selector_background)
                addView(ImageView(activity).apply {
                    setImageDrawable(app.icon)
                    importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
                }, LinearLayout.LayoutParams(activity.dp(40), activity.dp(40)).apply { marginEnd = activity.dp(16) })
                addView(LinearLayout(activity).apply {
                    orientation = LinearLayout.VERTICAL
                    addView(TextView(activity).apply {
                        text = app.label
                        textSize = 16f
                        setTextColor(ink)
                        setTypeface(typeface, Typeface.BOLD)
                    })
                    addView(TextView(activity).apply {
                        text = app.packageName
                        textSize = 12f
                        setTextColor(muted)
                        setPadding(0, activity.dp(3), 0, 0)
                        maxLines = 1
                        ellipsize = android.text.TextUtils.TruncateAt.END
                    })
                }, LinearLayout.LayoutParams(0, -2, 1f).apply { marginEnd = activity.dp(12) })
                addView(toggle, LinearLayout.LayoutParams(-2, -2))
                // Keep the switch as the single accessibility control for the row.
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
                isFocusable = false
                setOnClickListener { toggle.performClick() }
            }, LinearLayout.LayoutParams(-1, -2))
        }
        more.visibility = if (filtered.size > limit) View.VISIBLE else View.GONE
    }
    allowAll.setOnClickListener { settings.setAllowed(apps.map { it.packageName }, true); render() }
    blockAll.setOnClickListener { settings.setAllowed(apps.map { it.packageName }, false); render() }
    more.setOnClickListener { limit += 100; render() }
    search.addTextChangedListener(object : TextWatcher {
        override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) = Unit
        override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {
            limit = 100
            render()
        }
        override fun afterTextChanged(s: Editable?) = Unit
    })
    Thread {
        val result = runCatching { settings.apps() }
        runOnUiThread {
            if (!isFinishing && !isDestroyed) {
                result.onSuccess { apps = it; render() }
                    .onFailure { summary.text = "Could not load apps. Reopen Notifications to try again." }
            }
        }
    }.start()
}
