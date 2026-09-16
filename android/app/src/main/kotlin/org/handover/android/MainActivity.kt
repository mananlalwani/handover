package org.handover.android

import android.content.Intent
import android.os.Bundle
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.EditText
import android.content.BroadcastReceiver
import android.content.IntentFilter
import android.app.AlertDialog

class MainActivity : android.app.Activity() {
    private lateinit var status: TextView
    private var shownPairCode: String? = null
    private fun refreshStatus() {
        val peer = NativeTransport.trustedPeerFingerprint(this) ?: "No paired desktop"
        status.text = "Handover\n\nDevice identity: ${DeviceIdentityStore(this).deviceId}\n\nPaired desktop: $peer"
    }
    private val pairReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: android.content.Context, intent: Intent) {
            if (intent.action == NativeTransport.ACTION_PAIRED || intent.action == NativeTransport.ACTION_REVOKED) {
                refreshStatus()
                return
            }
            if (intent.action != NativeTransport.ACTION_PAIR_REQUEST) return
            val code = org.json.JSONObject(intent.getStringExtra(NativeTransport.EXTRA_JSON).orEmpty()).optString("code")
            showPairDialog(code)
        }
    }

    private fun showPairDialog(code: String) {
        if (code.isEmpty() || shownPairCode == code) return
        shownPairCode = code
        AlertDialog.Builder(this).setTitle("Pair Handover device")
            .setMessage("Confirm this code on Linux:\n\n$code")
            .setNegativeButton("Cancel", null)
            .setPositiveButton("Pair") { _, _ ->
                startService(Intent(this, HandoverForegroundService::class.java)
                    .setAction(HandoverForegroundService.ACTION_PAIR).putExtra(HandoverForegroundService.EXTRA_CODE, code))
            }.setOnDismissListener { shownPairCode = null }.show()
    }

    override fun onStart() {
        super.onStart()
        registerReceiver(pairReceiver, IntentFilter().apply {
            addAction(NativeTransport.ACTION_PAIR_REQUEST)
            addAction(NativeTransport.ACTION_PAIRED)
            addAction(NativeTransport.ACTION_REVOKED)
        }, RECEIVER_NOT_EXPORTED)
        refreshStatus()
        NativeTransport.pendingPairingCode(this)?.let(::showPairDialog)
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
        status = TextView(this).apply {
            setPadding(32, 48, 32, 24)
        }
        val start = Button(this).apply {
            text = "Enable Handover connection"
            setOnClickListener {
                val service = Intent(this@MainActivity, HandoverForegroundService::class.java)
                if (android.os.Build.VERSION.SDK_INT >= 26) startForegroundService(service) else startService(service)
            }
        }
        val revoke = Button(this).apply {
            text = "Unpair this desktop"
            setOnClickListener { startService(Intent(this@MainActivity, HandoverForegroundService::class.java).setAction(HandoverForegroundService.ACTION_REVOKE)) }
        }
        val address = EditText(this).apply {
            hint = "Linux address:port if discovery is blocked"
            inputType = android.text.InputType.TYPE_CLASS_TEXT
            setSingleLine(true)
        }
        val manualConnect = Button(this).apply {
            text = "Connect to address"
            setOnClickListener {
                val service = Intent(this@MainActivity, HandoverForegroundService::class.java)
                    .setAction(HandoverForegroundService.ACTION_CONNECT)
                    .putExtra(HandoverForegroundService.EXTRA_ADDRESS, address.text.toString().trim())
                startForegroundService(service)
            }
        }
        setContentView(LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            addView(status, LinearLayout.LayoutParams(-1, 0, 1f))
            addView(start, LinearLayout.LayoutParams(-1, -2))
            addView(address, LinearLayout.LayoutParams(-1, -2))
            addView(manualConnect, LinearLayout.LayoutParams(-1, -2))
            addView(revoke, LinearLayout.LayoutParams(-1, -2))
        })
    }

    companion object { private const val LOCAL_NETWORK_REQUEST = 42 }
}
