package org.handover.android

import android.graphics.Color
import android.graphics.Typeface
import android.view.View
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView

internal fun MainActivity.featurePage(
    title: String,
    description: String,
    vararg sections: View,
): LinearLayout {
    val activity = this
    return LinearLayout(activity).apply {
        orientation = LinearLayout.VERTICAL
        val back = activity.secondary(Button(activity).apply {
            text = "‹  Back"
            setOnClickListener { activity.homePage?.let(activity::showPage) }
        })
        addView(back, LinearLayout.LayoutParams(-2, -2))
        addView(TextView(activity).apply {
            text = title
            textSize = 28f
            setTextColor(Color.rgb(24, 30, 44))
            setTypeface(typeface, Typeface.BOLD)
            setPadding(0, activity.dp(10), 0, activity.dp(4))
        })
        addView(TextView(activity).apply {
            text = description
            textSize = 15f
            setTextColor(Color.rgb(92, 99, 116))
            setPadding(0, 0, 0, activity.dp(10))
        })
        sections.forEach { section ->
            addView(section, LinearLayout.LayoutParams(-1, -2).apply { topMargin = activity.dp(12) })
        }
    }
}
