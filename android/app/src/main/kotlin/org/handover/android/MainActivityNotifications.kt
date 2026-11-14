package org.handover.android

import android.graphics.Color
import android.text.Editable
import android.text.TextWatcher
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.Switch
import android.widget.TextView

internal fun MainActivity.openNotificationForwarding() {
    val activity = this
    val settings = NotificationForwarding(this)
    val search = EditText(this).apply {
        hint = "Search apps"
        setSingleLine(true)
    }
    val summary = TextView(this).apply { text = "Loading apps…" }
    val rows = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
    val more = secondary(Button(this).apply { text = "Show more apps" })
    val page = featurePage(
        "Notifications",
        "Choose which apps can send notifications to your laptop. Turning an app off removes its forwarded notifications without dismissing them on your phone. New apps are allowed by default.",
        panel(TextView(this).apply {
            text = if (HandoverNotificationService.isEnabled(activity)) {
                "Notification access is enabled. Changes apply to your paired laptop and are saved on this phone."
            } else {
                "Notification access is off. You can choose apps now, then enable access in Permissions."
            }
        }),
        panel(search, summary, rows, more),
        TextView(this).apply {
            text = "Includes launcher apps and apps that have posted notifications. If an app is missing, let it post a notification, then reopen this page. Use Mute in Messages to silence chats connected separately on your laptop."
            setTextColor(Color.rgb(92, 99, 116))
        },
    )
    showPage(page)
    var apps = emptyList<NotificationForwarding.App>()
    var limit = 100
    fun render() {
        val query = search.text.toString().trim()
        val filtered = apps.filter {
            it.label.contains(query, ignoreCase = true) || it.packageName.contains(query, ignoreCase = true)
        }
        rows.removeAllViews()
        summary.text = "${apps.count { settings.isAllowed(it.packageName) }} of ${apps.size} apps allowed · ${filtered.size} match"
        if (filtered.isEmpty()) {
            rows.addView(TextView(activity).apply { text = "No matching apps." })
        }
        filtered.take(limit).forEach { app ->
            rows.addView(Switch(activity).apply {
                text = "${app.label}\n${app.packageName}"
                textSize = 15f
                setPadding(0, activity.dp(12), 0, activity.dp(12))
                isChecked = settings.isAllowed(app.packageName)
                setOnCheckedChangeListener { _, allowed ->
                    settings.setAllowed(app.packageName, allowed)
                    summary.text = "${apps.count { settings.isAllowed(it.packageName) }} of ${apps.size} apps allowed · ${filtered.size} match"
                }
            }, LinearLayout.LayoutParams(-1, -2))
        }
        more.visibility = if (filtered.size > limit) android.view.View.VISIBLE else android.view.View.GONE
    }
    more.visibility = android.view.View.GONE
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
