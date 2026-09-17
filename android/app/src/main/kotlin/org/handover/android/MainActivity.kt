package org.handover.android

import android.content.Intent
import android.content.ComponentName
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Bundle
import android.os.PowerManager
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.EditText
import android.content.BroadcastReceiver
import android.content.IntentFilter
import android.app.AlertDialog
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.RemoteInput
import android.provider.Settings
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.view.View

class MainActivity : android.app.Activity() {
    private lateinit var status: TextView
    private lateinit var notificationStatus: TextView
    private lateinit var mediaStatus: TextView
    private lateinit var capabilitiesStatus: TextView
    private var shownPairCode: String? = null
    private var pairDialog: AlertDialog? = null
    private var testCounter = 1

    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()

    private fun panel(vararg children: View) = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(dp(18), dp(16), dp(18), dp(16))
        background = GradientDrawable().apply {
            setColor(Color.rgb(247, 248, 252))
            cornerRadius = dp(18).toFloat()
            setStroke(dp(1), Color.rgb(224, 227, 235))
        }
        children.forEach { addView(it, LinearLayout.LayoutParams(-1, -2)) }
    }

    private fun sectionTitle(text: String) = TextView(this).apply {
        this.text = text
        textSize = 18f
        setTextColor(Color.rgb(28, 34, 48))
        setTypeface(typeface, Typeface.BOLD)
        setPadding(0, 0, 0, dp(10))
    }

    private fun primary(button: Button) = button.apply {
        isAllCaps = false
        textSize = 15f
        setTextColor(Color.WHITE)
        backgroundTintList = android.content.res.ColorStateList.valueOf(Color.rgb(48, 84, 210))
    }

    private fun secondary(button: Button) = button.apply {
        isAllCaps = false
        textSize = 14f
    }
    private fun permissionLabel(permission: String): String =
        if (CallController.hasPermission(this, permission)) "granted" else "not granted"

    private fun refreshStatus() {
        val peer = NativeTransport.trustedPeerFingerprint(this) ?: "No paired desktop"
        status.text = "Handover\n\nDevice identity: ${DeviceIdentityStore(this).deviceId}\n\nPaired desktop: $peer"
        val listener = if (HandoverNotificationService.isEnabled(this)) "granted" else "not granted"
        val reply = TestNotificationReceiver.lastReply(this)?.let { "\nLast test reply: $it" }.orEmpty()
        notificationStatus.text = "Notification access: $listener$reply"
        mediaStatus.text = if (TestMediaSession.isActive()) "Test media: playing" else "Test media: stopped"
        val notifications = getSystemService(NotificationManager::class.java).areNotificationsEnabled()
        val localNetwork = if (android.os.Build.VERSION.SDK_INT < 37 ||
            checkSelfPermission("android.permission.ACCESS_LOCAL_NETWORK") == PackageManager.PERMISSION_GRANTED
        ) "granted" else "not granted"
        val battery = getSystemService(PowerManager::class.java)
            .isIgnoringBatteryOptimizations(packageName)
        capabilitiesStatus.text = buildString {
            append("Permissions & capabilities\n\n")
            append("Notification access: $listener\n")
            append("App notifications: ${if (notifications) "granted" else "not granted"}\n")
            append("Local network: $localNetwork\n")
            append("Battery optimization: ${if (battery) "unrestricted" else "optimized"}\n")
            append("Files/photos: system picker (on demand)\n")
            append("Media controls: notification access\n")
            append("Call state: ${permissionLabel(android.Manifest.permission.READ_PHONE_STATE)}\n")
            append("Place calls: ${permissionLabel(android.Manifest.permission.CALL_PHONE)}\n")
            append("Answer/end calls: ${permissionLabel(android.Manifest.permission.ANSWER_PHONE_CALLS)}")
        }
    }
    private val pairReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: android.content.Context, intent: Intent) {
            if (intent.action == NativeTransport.ACTION_PAIRED || intent.action == NativeTransport.ACTION_REVOKED) {
                dismissPairDialog()
                refreshStatus()
                return
            }
            if (intent.action == TestNotificationReceiver.ACTION_TEST_REPLY_RECEIVED) {
                refreshStatus()
                return
            }
            if (intent.action != NativeTransport.ACTION_PAIR_REQUEST) return
            val code = org.json.JSONObject(intent.getStringExtra(NativeTransport.EXTRA_JSON).orEmpty()).optString("code")
            showPairDialog(code)
        }
    }

    private fun showPairDialog(code: String) {
        if (code.isEmpty()) return
        if (shownPairCode == code && pairDialog?.isShowing == true) return
        // Each ceremony derives a fresh code, so a new ceremony replaces the
        // previous dialog instead of stacking prompts.
        shownPairCode = code
        pairDialog?.dismiss()
        pairDialog = AlertDialog.Builder(this).setTitle("Pair Handover device")
            .setMessage("Confirm this code on Linux:\n\n$code")
            .setNegativeButton("Cancel", null)
            .setPositiveButton("Pair") { _, _ ->
                startService(Intent(this, HandoverForegroundService::class.java)
                    .setAction(HandoverForegroundService.ACTION_PAIR).putExtra(HandoverForegroundService.EXTRA_CODE, code))
            }.setOnDismissListener { shownPairCode = null; pairDialog = null }.show()
    }

    private fun dismissPairDialog() {
        pairDialog?.dismiss()
        pairDialog = null
        shownPairCode = null
    }

    /** Harmless local notification exercising the native post/update/remove
     * path. It carries a genuine RemoteInput reply action so the desktop
     * inline-reply path has something real to target. */
    private fun postTestNotification(updated: Boolean) {
        if (android.os.Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) !=
            android.content.pm.PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(android.Manifest.permission.POST_NOTIFICATIONS), POST_NOTIFICATIONS_REQUEST)
        }
        TestNotifications.post(this, updated, testCounter++)
    }

    override fun onStart() {
        super.onStart()
        registerReceiver(pairReceiver, IntentFilter().apply {
            addAction(NativeTransport.ACTION_PAIR_REQUEST)
            addAction(NativeTransport.ACTION_PAIRED)
            addAction(NativeTransport.ACTION_REVOKED)
            addAction(TestNotificationReceiver.ACTION_TEST_REPLY_RECEIVED)
        }, RECEIVER_NOT_EXPORTED)
        refreshStatus()
        NativeTransport.pendingPairingCode(this)?.let(::showPairDialog)
    }

    override fun onResume() {
        super.onResume()
        refreshStatus()
        HandoverForegroundService.refreshCallsIfRunning()
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == CALL_PERMISSIONS_REQUEST) {
            refreshStatus()
            HandoverForegroundService.refreshCallsIfRunning()
            if (permissions.any { !CallController.hasPermission(this, it) }) {
                AlertDialog.Builder(this).setTitle("Call permissions not granted")
                    .setMessage("Review the permission status above. If Android no longer shows a prompt, enable Phone access in app settings.")
                    .setPositiveButton("App settings") { _, _ ->
                        startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS)
                            .setData(Uri.parse("package:$packageName")))
                    }.setNegativeButton("Close", null).show()
            }
        }
    }

    override fun onStop() {
        unregisterReceiver(pairReceiver)
        super.onStop()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        handleShareIntent(intent)
        if (android.os.Build.VERSION.SDK_INT >= 37 &&
            checkSelfPermission("android.permission.ACCESS_LOCAL_NETWORK") != android.content.pm.PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf("android.permission.ACCESS_LOCAL_NETWORK"), LOCAL_NETWORK_REQUEST)
        }
        status = TextView(this).apply {
            textSize = 14f
            setTextColor(Color.rgb(70, 77, 94))
        }
        notificationStatus = TextView(this).apply {
            textSize = 14f
            setTextColor(Color.rgb(70, 77, 94))
        }
        mediaStatus = TextView(this).apply {
            textSize = 14f
            setTextColor(Color.rgb(70, 77, 94))
        }
        capabilitiesStatus = TextView(this).apply {
            textSize = 14f
            setTextColor(Color.rgb(70, 77, 94))
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
        val notificationAccess = Button(this).apply {
            text = "Enable notification access"
            setOnClickListener {
                val component = ComponentName(
                    this@MainActivity, HandoverNotificationService::class.java,
                )
                val detail = Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS)
                    .putExtra(Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME, component.flattenToString())
                runCatching { startActivity(detail) }.getOrElse {
                    startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS))
                }
            }
        }
        val appNotificationAccess = Button(this).apply {
            text = "Enable app notifications"
            setOnClickListener {
                if (android.os.Build.VERSION.SDK_INT >= 33 &&
                    checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
                ) {
                    requestPermissions(
                        arrayOf(android.Manifest.permission.POST_NOTIFICATIONS),
                        POST_NOTIFICATIONS_REQUEST,
                    )
                } else {
                    startActivity(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                        .putExtra(Settings.EXTRA_APP_PACKAGE, packageName))
                }
            }
        }
        val localNetworkAccess = Button(this).apply {
            text = "Enable local network"
            setOnClickListener {
                if (android.os.Build.VERSION.SDK_INT >= 37) {
                    requestPermissions(arrayOf("android.permission.ACCESS_LOCAL_NETWORK"), LOCAL_NETWORK_REQUEST)
                }
            }
        }
        val batteryAccess = Button(this).apply {
            text = "Review battery optimization"
            setOnClickListener {
                startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
            }
        }
        val backgroundAccess = Button(this).apply {
            text = "Review background data and app access"
            setOnClickListener {
                startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS)
                    .setData(Uri.parse("package:$packageName")))
            }
        }
        val callAccess = Button(this).apply {
            text = "Enable call controls"
            setOnClickListener {
                val missing = arrayOf(
                    android.Manifest.permission.READ_PHONE_STATE,
                    android.Manifest.permission.CALL_PHONE,
                    android.Manifest.permission.ANSWER_PHONE_CALLS,
                ).filterNot { CallController.hasPermission(this@MainActivity, it) }
                if (missing.isEmpty()) {
                    refreshStatus()
                    android.widget.Toast.makeText(this@MainActivity,
                        "All call permissions are already granted", android.widget.Toast.LENGTH_LONG).show()
                    HandoverForegroundService.refreshCallsIfRunning()
                } else {
                    requestPermissions(missing.toTypedArray(), CALL_PERMISSIONS_REQUEST)
                }
            }
        }
        val postTest = Button(this).apply {
            text = "Post test notification"
            setOnClickListener { postTestNotification(false) }
        }
        val updateTest = Button(this).apply {
            text = "Update test notification"
            setOnClickListener { postTestNotification(true) }
        }
        val removeTest = Button(this).apply {
            text = "Remove test notification"
            setOnClickListener { TestNotifications.remove(this@MainActivity) }
        }
        val startTestMedia = Button(this).apply {
            text = "Start test media"
            setOnClickListener {
                TestMediaSession.start(this@MainActivity)
                refreshStatus()
            }
        }
        val stopTestMedia = Button(this).apply {
            text = "Stop test media"
            setOnClickListener {
                TestMediaSession.stop()
                refreshStatus()
            }
        }
        primary(start)
        secondary(manualConnect)
        listOf(notificationAccess, appNotificationAccess, localNetworkAccess, batteryAccess,
            backgroundAccess, callAccess, postTest, updateTest, removeTest, startTestMedia,
            stopTestMedia, revoke).forEach(::secondary)

        val diagnostics = panel(
            sectionTitle("Diagnostics"), postTest, updateTest, removeTest,
            startTestMedia, stopTestMedia,
        ).apply { visibility = View.GONE }
        val diagnosticsToggle = secondary(Button(this).apply {
            text = "Show diagnostics"
            setOnClickListener {
                diagnostics.visibility = if (diagnostics.visibility == View.VISIBLE) View.GONE else View.VISIBLE
                text = if (diagnostics.visibility == View.VISIBLE) "Hide diagnostics" else "Show diagnostics"
            }
        })
        val title = TextView(this).apply {
            text = "Handover"
            textSize = 32f
            setTextColor(Color.rgb(24, 30, 44))
            setTypeface(typeface, Typeface.BOLD)
        }
        val subtitle = TextView(this).apply {
            text = "Your phone, available on Linux"
            textSize = 16f
            setTextColor(Color.rgb(92, 99, 116))
            setPadding(0, dp(2), 0, dp(18))
        }
        val content = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(24), dp(20), dp(32))
            addView(title)
            addView(subtitle)
            listOf(
                panel(sectionTitle("Connection"), status, start, address, manualConnect),
                panel(sectionTitle("Permissions & capabilities"), capabilitiesStatus,
                    notificationAccess, appNotificationAccess, callAccess, localNetworkAccess,
                    batteryAccess, backgroundAccess),
                panel(sectionTitle("Activity"), notificationStatus, mediaStatus),
                diagnosticsToggle,
                diagnostics,
                panel(sectionTitle("Pairing"), revoke),
            ).forEach { view ->
                addView(view, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(12) })
            }
        }
        setContentView(ScrollView(this).apply {
            setBackgroundColor(Color.rgb(238, 241, 247))
            addView(content)
        })
    }

    override fun onNewIntent(intent: Intent?) {
        super.onNewIntent(intent)
        if (intent != null) handleShareIntent(intent)
    }

    private fun handleShareIntent(intent: Intent) {
        if (intent.action != Intent.ACTION_SEND) return
        val service = Intent(this, HandoverForegroundService::class.java)
        when {
            intent.type == "text/plain" -> {
                val text = intent.getStringExtra(Intent.EXTRA_TEXT) ?: return
                service.action = HandoverForegroundService.ACTION_SHARE_URL
                service.putExtra(HandoverForegroundService.EXTRA_URL, text)
            }
            intent.getParcelableExtra<android.net.Uri>(Intent.EXTRA_STREAM) != null -> {
                service.action = HandoverForegroundService.ACTION_SHARE_FILE
                service.putExtra(HandoverForegroundService.EXTRA_URI,
                    intent.getParcelableExtra<android.net.Uri>(Intent.EXTRA_STREAM))
                addUriPermission(intent, service)
            }
            else -> return
        }
        val target = NativeTransport.trustedPeerFingerprint(this) ?: return
        AlertDialog.Builder(this)
            .setTitle("Send with Handover")
            .setMessage("Send to paired desktop $target?")
            .setNegativeButton("Cancel", null)
            .setPositiveButton("Send") { _, _ ->
                if (android.os.Build.VERSION.SDK_INT >= 26) startForegroundService(service) else startService(service)
            }.show()
    }

    private fun addUriPermission(source: Intent, destination: Intent) {
        val uri = source.getParcelableExtra<android.net.Uri>(Intent.EXTRA_STREAM) ?: return
        if ((source.flags and Intent.FLAG_GRANT_READ_URI_PERMISSION) != 0) {
            destination.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            destination.clipData = android.content.ClipData.newUri(contentResolver, "Shared file", uri)
        }
    }

    companion object {
        private const val LOCAL_NETWORK_REQUEST = 42
        private const val POST_NOTIFICATIONS_REQUEST = 43
        private const val CALL_PERMISSIONS_REQUEST = 45
    }
}
