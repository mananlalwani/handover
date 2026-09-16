package org.handover.android

import android.content.Intent
import android.os.Bundle
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import android.content.BroadcastReceiver
import android.content.IntentFilter
import android.app.AlertDialog

class MainActivity : android.app.Activity() {
    private val pairReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: android.content.Context, intent: Intent) {
            if (intent.action != NativeTransport.ACTION_PAIR_REQUEST) return
            val code = org.json.JSONObject(intent.getStringExtra(NativeTransport.EXTRA_JSON).orEmpty()).optString("code")
            AlertDialog.Builder(this@MainActivity).setTitle("Pair Handover device")
                .setMessage("Confirm this code on Linux:\n\n$code")
                .setNegativeButton("Cancel", null)
                .setPositiveButton("Pair") { _, _ ->
                    startService(Intent(this@MainActivity, HandoverForegroundService::class.java)
                        .setAction(HandoverForegroundService.ACTION_PAIR).putExtra(HandoverForegroundService.EXTRA_CODE, code))
                }.show()
        }
    }

    override fun onStart() {
        super.onStart()
        registerReceiver(pairReceiver, IntentFilter(NativeTransport.ACTION_PAIR_REQUEST), RECEIVER_NOT_EXPORTED)
    }

    override fun onStop() {
        unregisterReceiver(pairReceiver)
        super.onStop()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (android.os.Build.VERSION.SDK_INT >= 37 &&
            checkSelfPermission("android.permission.ACCESS_LOCAL_NETWORK") != android.content.pm.PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf("android.permission.ACCESS_LOCAL_NETWORK"), LOCAL_NETWORK_REQUEST)
        }
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
        val revoke = Button(this).apply {
            text = "Unpair this desktop"
            setOnClickListener { startService(Intent(this@MainActivity, HandoverForegroundService::class.java).setAction(HandoverForegroundService.ACTION_REVOKE)) }
        }
        setContentView(LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            addView(status, LinearLayout.LayoutParams(-1, 0, 1f))
            addView(start, LinearLayout.LayoutParams(-1, -2))
            addView(revoke, LinearLayout.LayoutParams(-1, -2))
        })
    }

    companion object { private const val LOCAL_NETWORK_REQUEST = 42 }
}
