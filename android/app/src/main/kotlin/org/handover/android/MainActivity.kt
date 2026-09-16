package org.handover.android

import android.content.Intent
import android.os.Bundle
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView

class MainActivity : android.app.Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val identity = DeviceIdentityStore(this).deviceId
        val status = TextView(this).apply {
            text = "Handover\n\nDevice identity: $identity\n\nNative connection service is stopped."
            setPadding(32, 48, 32, 24)
        }
        val start = Button(this).apply {
            text = "Enable Handover connection"
            setOnClickListener {
                val service = Intent(this@MainActivity, HandoverForegroundService::class.java)
                if (android.os.Build.VERSION.SDK_INT >= 26) startForegroundService(service) else startService(service)
                status.text = "Handover\n\nDevice identity: $identity\n\nConnection service is running."
            }
        }
        setContentView(LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            addView(status, LinearLayout.LayoutParams(-1, 0, 1f))
            addView(start, LinearLayout.LayoutParams(-1, -2))
        })
    }
}
