package org.handover.android

import android.annotation.SuppressLint
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
import android.widget.Switch
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
import android.view.MotionEvent
import java.text.DateFormat
import java.util.Date

// This compact control surface intentionally formats runtime state inline:
// identity, permissions, and transfer metadata are not secrets and are not
// logged or persisted by the activity.
@SuppressLint("SetTextI18n", "InlinedApi", "UnspecifiedRegisterReceiverFlag", "GestureBackNavigation")
class MainActivity : android.app.Activity() {
    private lateinit var status: TextView
    private lateinit var notificationStatus: TextView
    private lateinit var mediaStatus: TextView
    private lateinit var transferStatus: LinearLayout
    private lateinit var capabilitiesStatus: TextView
    private lateinit var updateStatus: TextView
    private lateinit var homeConnect: Button
    private lateinit var pageHost: LinearLayout
    private var homePage: View? = null
    private var shownPairCode: String? = null
    private var pairDialog: AlertDialog? = null
    private var testCounter = 1
    private var reconnectPromptShown = false
    private var connectionState = "offline"
    private val permissionButtons = mutableListOf<Pair<Button, () -> Boolean>>()

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

    private fun refreshPermissionButtons() {
        permissionButtons.forEach { (button, granted) ->
            val enabled = granted()
            button.backgroundTintList = android.content.res.ColorStateList.valueOf(
                if (enabled) Color.rgb(35, 142, 84) else Color.rgb(105, 70, 190),
            )
            button.setTextColor(Color.WHITE)
            button.alpha = if (enabled) 0.88f else 1f
        }
    }

    private fun startHandoverConnection() {
        val service = Intent(this, HandoverForegroundService::class.java)
        startForegroundService(service)
        AppUpdater.clearReconnectNeeded(this)
        refreshStatus()
    }

    private fun menuButton(title: String, description: String, action: () -> Unit) =
        TextView(this).apply {
            text = "$title\n$description"
            textSize = 16f
            setTextColor(Color.rgb(28, 34, 48))
            setPadding(dp(18), dp(16), dp(18), dp(16))
            background = GradientDrawable().apply {
                setColor(Color.WHITE)
                cornerRadius = dp(18).toFloat()
                setStroke(dp(1), Color.rgb(224, 227, 235))
            }
            isClickable = true
            isFocusable = true
            foreground = getDrawable(android.R.drawable.list_selector_background)
            setOnClickListener { action() }
        }

    private fun showPage(page: View) {
        pageHost.removeAllViews()
        (page.parent as? android.view.ViewGroup)?.removeView(page)
        pageHost.addView(page, LinearLayout.LayoutParams(-1, -2))
    }

    private fun handleBackNavigation() {
        val home = homePage
        if (home != null && pageHost.childCount > 0 && pageHost.getChildAt(0) !== home) {
            showPage(home)
        } else {
            finish()
        }
    }

    override fun onKeyDown(keyCode: Int, event: android.view.KeyEvent): Boolean {
        if (keyCode == android.view.KeyEvent.KEYCODE_BACK) {
            handleBackNavigation()
            return true
        }
        return super.onKeyDown(keyCode, event)
    }
    private fun permissionLabel(permission: String): String =
        if (CallController.hasPermission(this, permission)) "granted" else "not granted"

    private fun refreshStatus() {
        val peer = NativeTransport.trustedPeerFingerprint(this) ?: "No paired desktop"
        status.text = "Connection: ${connectionState.replaceFirstChar { it.uppercase() }}\n\nDevice identity: ${DeviceIdentityStore(this).deviceId}\n\nPaired desktop: $peer"
        val listener = if (HandoverNotificationService.isEnabled(this)) "granted" else "not granted"
        val reply = TestNotificationReceiver.lastReply(this)?.let { "\nLast test reply: $it" }.orEmpty()
        notificationStatus.text = "Notification access: $listener$reply"
        mediaStatus.text = if (TestMediaSession.isActive()) "Test media: playing" else "Test media: stopped"
        refreshTransferHistory()
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
        val installed = packageManager.getPackageInfo(packageName, 0)
        val pending = AppUpdater.pending(this)
        updateStatus.text = if (pending == null) {
            "Installed: ${installed.versionName}\nVerified desktop updates: active\nNo downloaded update"
        } else {
            "Installed: ${installed.versionName}\nReady to install: ${pending.versionName.ifEmpty { pending.versionCode.toString() }}"
        }
        if (::homeConnect.isInitialized) {
            val reconnect = AppUpdater.reconnectNeeded(this)
            homeConnect.text = if (reconnect) "Reconnect after update" else "Connect / Pair desktop"
            homeConnect.backgroundTintList = android.content.res.ColorStateList.valueOf(
                if (reconnect) Color.rgb(232, 139, 22) else Color.rgb(48, 84, 210),
            )
        }
        refreshPermissionButtons()
    }

    private fun refreshTransferHistory() {
        if (!::transferStatus.isInitialized) return
        transferStatus.removeAllViews()
        val transfers = TransferHistory.read(this)
        if (transfers.isEmpty()) {
            transferStatus.addView(TextView(this).apply {
                text = "No transfers"
                textSize = 14f
                setTextColor(Color.rgb(70, 77, 94))
            })
            return
        }
        transfers.forEachIndexed { index, record ->
            val label = record.name.takeUnless { it == record.id }
                ?: "Transfer ${record.id.take(8)}…"
            val timestamp = if (record.timestamp > 0) {
                DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT)
                    .format(Date(record.timestamp))
            } else "Time unavailable"
            val details = TextView(this).apply {
                text = "${record.kind.replaceFirstChar { it.uppercase() }} · ${record.status.replaceFirstChar { it.uppercase() }}\n" +
                    "${label.take(120)}\n$timestamp"
                textSize = 14f
                setTextColor(Color.rgb(28, 34, 48))
            }
            val actions = LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                val open = secondary(Button(this@MainActivity).apply {
                    text = "Open"
                    isEnabled = record.uri != null
                    setOnClickListener {
                        if (!TransferHistory.open(this@MainActivity, record)) {
                            android.widget.Toast.makeText(this@MainActivity,
                                "This transfer cannot be opened here", android.widget.Toast.LENGTH_LONG).show()
                        }
                    }
                })
                val remove = secondary(Button(this@MainActivity).apply {
                    text = "Remove"
                    setOnClickListener {
                        TransferHistory.remove(this@MainActivity, record.id)
                        refreshStatus()
                    }
                })
                addView(open, LinearLayout.LayoutParams(0, -2, 1f))
                addView(remove, LinearLayout.LayoutParams(0, -2, 1f))
            }
            transferStatus.addView(LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(0, if (index == 0) 0 else dp(12), 0, dp(8))
                addView(details, LinearLayout.LayoutParams(-1, -2))
                addView(actions, LinearLayout.LayoutParams(-1, -2))
            }, LinearLayout.LayoutParams(-1, -2))
        }
    }

    private fun scanForUpdates() {
        AppUpdater.scanDownloads(this)
        refreshStatus()
        android.widget.Toast.makeText(this, "Downloads/Handover scanned", android.widget.Toast.LENGTH_SHORT).show()
    }
    private val pairReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: android.content.Context, intent: Intent) {
            if (intent.action == NativeTransport.ACTION_PAIRED || intent.action == NativeTransport.ACTION_REVOKED) {
                dismissPairDialog()
                refreshStatus()
                return
            }
            if (intent.action == NativeTransport.ACTION_CONNECTION_STATE) {
                connectionState = org.json.JSONObject(
                    intent.getStringExtra(NativeTransport.EXTRA_JSON).orEmpty(),
                ).optString("state", "offline")
                refreshStatus()
                return
            }
            if (intent.action == TestNotificationReceiver.ACTION_TEST_REPLY_RECEIVED) {
                refreshStatus()
                return
            }
            if (intent.action == NativeTransport.ACTION_SHARE_RECEIVED) {
                refreshStatus()
                return
            }
            if (intent.action == NativeTransport.ACTION_TRANSFER_RESULT) {
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
        val filter = IntentFilter().apply {
            addAction(NativeTransport.ACTION_PAIR_REQUEST)
            addAction(NativeTransport.ACTION_PAIRED)
            addAction(NativeTransport.ACTION_REVOKED)
            addAction(NativeTransport.ACTION_CONNECTION_STATE)
            addAction(TestNotificationReceiver.ACTION_TEST_REPLY_RECEIVED)
            addAction(NativeTransport.ACTION_SHARE_RECEIVED)
            addAction(NativeTransport.ACTION_TRANSFER_RESULT)
        }
        if (android.os.Build.VERSION.SDK_INT >= 33) {
            registerReceiver(pairReceiver, filter, RECEIVER_NOT_EXPORTED)
        } else {
            @Suppress("DEPRECATION")
            registerReceiver(pairReceiver, filter)
        }
        AppUpdater.scanDownloads(this)
        refreshStatus()
        NativeTransport.pendingPairingCode(this)?.let(::showPairDialog)
        if (AppUpdater.reconnectNeeded(this) && !reconnectPromptShown) {
            reconnectPromptShown = true
            AlertDialog.Builder(this).setTitle("Reconnect Handover")
                .setMessage("The update was installed successfully. Reconnect to your already-paired Linux desktop now?")
                .setPositiveButton("Reconnect") { _, _ -> startHandoverConnection() }
                .setNegativeButton("Later", null)
                .show()
        }
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
        if (android.os.Build.VERSION.SDK_INT >= 33) {
            onBackInvokedDispatcher.registerOnBackInvokedCallback(
                android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT,
            ) { handleBackNavigation() }
        }
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
        transferStatus = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
        }
        capabilitiesStatus = TextView(this).apply {
            textSize = 14f
            setTextColor(Color.rgb(70, 77, 94))
        }
        updateStatus = TextView(this).apply {
            textSize = 14f
            setTextColor(Color.rgb(70, 77, 94))
        }
        val start = Button(this).apply {
            text = "Enable Handover connection"
            setOnClickListener { startHandoverConnection() }
        }
        homeConnect = primary(Button(this).apply {
            text = "Connect / Pair desktop"
            setOnClickListener { startHandoverConnection() }
        })
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
        permissionButtons += notificationAccess to {
            HandoverNotificationService.isEnabled(this@MainActivity)
        }
        permissionButtons += appNotificationAccess to {
            getSystemService(NotificationManager::class.java).areNotificationsEnabled()
        }
        permissionButtons += callAccess to {
            listOf(android.Manifest.permission.READ_PHONE_STATE, android.Manifest.permission.CALL_PHONE,
                android.Manifest.permission.ANSWER_PHONE_CALLS).all {
                CallController.hasPermission(this@MainActivity, it)
            }
        }
        permissionButtons += localNetworkAccess to {
            android.os.Build.VERSION.SDK_INT < 37 ||
                checkSelfPermission("android.permission.ACCESS_LOCAL_NETWORK") == PackageManager.PERMISSION_GRANTED
        }
        permissionButtons += batteryAccess to {
            getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(packageName)
        }
        refreshPermissionButtons()

        fun page(title: String, description: String, vararg sections: View) =
            LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                val back = secondary(Button(this@MainActivity).apply {
                    text = "‹  Back"
                    setOnClickListener { homePage?.let(::showPage) }
                })
                addView(back, LinearLayout.LayoutParams(-2, -2))
                addView(TextView(this@MainActivity).apply {
                    text = title
                    textSize = 28f
                    setTextColor(Color.rgb(24, 30, 44))
                    setTypeface(typeface, Typeface.BOLD)
                    setPadding(0, dp(10), 0, dp(4))
                })
                addView(TextView(this@MainActivity).apply {
                    text = description
                    textSize = 15f
                    setTextColor(Color.rgb(92, 99, 116))
                    setPadding(0, 0, 0, dp(10))
                })
                sections.forEach { section ->
                    addView(section, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(12) })
                }
            }

        val connectionPage = page(
            "Connection", "Connect this phone to your trusted Linux desktop.",
            panel(sectionTitle("Handover service"), start),
            panel(sectionTitle("Manual connection"), address, manualConnect),
            panel(sectionTitle("Pairing"), revoke),
        )
        val permissionsPage = page(
            "Permissions", "Enable only the capabilities you want Handover to provide.",
            panel(sectionTitle("Current access"), capabilitiesStatus),
            panel(sectionTitle("Notifications"), notificationAccess, appNotificationAccess),
            panel(sectionTitle("Calls"), callAccess),
            panel(sectionTitle("Connectivity & background"), localNetworkAccess,
                batteryAccess, backgroundAccess),
            panel(sectionTitle("Clipboard"), Switch(this).apply {
                text = "Sync clipboard in the foreground service"
                isChecked = HandoverForegroundService.clipboardSyncEnabled()
                setOnCheckedChangeListener { _, enabled ->
                    HandoverForegroundService.setClipboardSync(enabled)
                }
            }),
        )
        val activityPage = page(
            "Activity", "Current phone-side Handover activity.",
            panel(sectionTitle("Notifications"), notificationStatus),
            panel(sectionTitle("Media"), mediaStatus),
            panel(sectionTitle("Recent transfers"), transferStatus,
                secondary(Button(this).apply {
                    text = "Clear all"
                    setOnClickListener { TransferHistory.clear(this@MainActivity); refreshStatus() }
                })),
        )
        var pointerX = 0f
        var pointerY = 0f
        var pointerMoved = false
        val pointerPad = View(this).apply {
            minimumHeight = dp(220)
            background = GradientDrawable().apply {
                setColor(Color.rgb(225, 229, 239))
                cornerRadius = dp(14).toFloat()
            }
            setOnTouchListener { _, event ->
                when (event.actionMasked) {
                    MotionEvent.ACTION_DOWN -> {
                        pointerX = event.x
                        pointerY = event.y
                        pointerMoved = false
                        true
                    }
                    MotionEvent.ACTION_MOVE -> {
                        val dx = ((event.x - pointerX) / 2f).toInt()
                        val dy = ((event.y - pointerY) / 2f).toInt()
                        if (dx != 0 || dy != 0) {
                            pointerMoved = true
                            HandoverForegroundService.presentation("pointer_move", dx, dy)
                            pointerX = event.x
                            pointerY = event.y
                        }
                        true
                    }
                    MotionEvent.ACTION_UP -> {
                        if (!pointerMoved) {
                            HandoverForegroundService.presentation("pointer_click")
                        }
                        true
                    }
                    else -> true
                }
            }
        }
        val presentationPage = page(
            "Presentation remote", "Control the active presentation on Linux.",
            panel(sectionTitle("Slides"), LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                addView(secondary(Button(this@MainActivity).apply {
                    text = "Previous"
                    setOnClickListener { HandoverForegroundService.presentation("previous") }
                }), LinearLayout.LayoutParams(0, -2, 1f))
                addView(secondary(Button(this@MainActivity).apply {
                    text = "Next"
                    setOnClickListener { HandoverForegroundService.presentation("next") }
                }), LinearLayout.LayoutParams(0, -2, 1f))
            }, LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                addView(secondary(Button(this@MainActivity).apply {
                    text = "Start"
                    setOnClickListener { HandoverForegroundService.presentation("start") }
                }), LinearLayout.LayoutParams(0, -2, 1f))
                addView(secondary(Button(this@MainActivity).apply {
                    text = "Stop"
                    setOnClickListener { HandoverForegroundService.presentation("stop") }
                }), LinearLayout.LayoutParams(0, -2, 1f))
                addView(secondary(Button(this@MainActivity).apply {
                    text = "Fullscreen"
                    setOnClickListener { HandoverForegroundService.presentation("fullscreen") }
                }), LinearLayout.LayoutParams(0, -2, 1f))
            }),
            panel(sectionTitle("Pointer"), TextView(this).apply {
                text = "Drag to move. Tap to click."
                setTextColor(Color.rgb(70, 77, 94))
            }, pointerPad),
        )
        val diagnosticsPage = page(
            "Diagnostics", "Local test tools. These do not contact anyone.",
            panel(sectionTitle("Notification test"), postTest, updateTest, removeTest),
            panel(sectionTitle("Media test"), startTestMedia, stopTestMedia),
        )
        val installUpdate = primary(Button(this).apply {
            text = "Install downloaded update"
            setOnClickListener {
                val update = AppUpdater.pending(this@MainActivity)
                if (update == null) {
                    android.widget.Toast.makeText(this@MainActivity,
                        "No verified update is ready", android.widget.Toast.LENGTH_LONG).show()
                } else {
                    AppUpdater.requestInstall(this@MainActivity, update)
                }
            }
        })
        val scanUpdates = secondary(Button(this).apply {
            text = "Scan for updates"
            setOnClickListener { scanForUpdates() }
        })
        val updatesPage = page(
            "Updates", "Updates received from your desktop are verified before installation.",
            panel(sectionTitle("App version"), updateStatus, scanUpdates, installUpdate),
            panel(sectionTitle("Security"), TextView(this).apply {
                text = "Only a newer Handover APK signed by the same certificate is accepted. Android always asks before installing it."
                textSize = 14f
                setTextColor(Color.rgb(70, 77, 94))
            }),
        )
        val volumePage = page(
            "System volume", "Control the default Linux audio output.",
            panel(sectionTitle("Volume"), LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                addView(secondary(Button(this@MainActivity).apply {
                    text = "− 5%"
                    setOnClickListener { HandoverForegroundService.volume("down") }
                }), LinearLayout.LayoutParams(0, -2, 1f))
                addView(secondary(Button(this@MainActivity).apply {
                    text = "+ 5%"
                    setOnClickListener { HandoverForegroundService.volume("up") }
                }), LinearLayout.LayoutParams(0, -2, 1f))
                addView(secondary(Button(this@MainActivity).apply {
                    text = "Mute"
                    setOnClickListener { HandoverForegroundService.volume("toggle_mute") }
                }), LinearLayout.LayoutParams(0, -2, 1f))
            }),
        )
        val contactsPage = page(
            "Contacts", "Send a fresh, on-demand contacts snapshot to Linux.",
            panel(sectionTitle("Privacy"), TextView(this).apply {
                text = "Contacts are read only when you request a sync and are not retained in phone-side history."
                setTextColor(Color.rgb(70, 77, 94))
            }, secondary(Button(this).apply {
                text = "Allow contacts access"
                setOnClickListener {
                    requestPermissions(arrayOf(android.Manifest.permission.READ_CONTACTS), CONTACTS_PERMISSION_REQUEST)
                }
            }), secondary(Button(this).apply {
                text = "Sync contacts now"
                setOnClickListener {
                    if (checkSelfPermission(android.Manifest.permission.READ_CONTACTS) == PackageManager.PERMISSION_GRANTED) {
                        HandoverForegroundService.syncContacts()
                    } else {
                        requestPermissions(arrayOf(android.Manifest.permission.READ_CONTACTS), CONTACTS_PERMISSION_REQUEST)
                    }
                }
            }), secondary(Button(this).apply {
                text = "Export provider diagnostic"
                setOnClickListener {
                    val uri = ContactsDump.export(this@MainActivity)
                    if (uri == null) {
                        android.widget.Toast.makeText(this@MainActivity,
                            "Could not export contact provider data", android.widget.Toast.LENGTH_LONG).show()
                    } else {
                        startActivity(Intent.createChooser(Intent(Intent.ACTION_SEND).apply {
                            type = "application/json"
                            putExtra(Intent.EXTRA_STREAM, uri)
                            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                        }, "Share contact provider diagnostic"))
                    }
                }
            }), secondary(Button(this).apply {
                text = "Send current clipboard to Linux"
                setOnClickListener { HandoverForegroundService.sendClipboard() }
            })),
        )

        val home = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            addView(TextView(this@MainActivity).apply {
                text = "Handover"
                textSize = 34f
                setTextColor(Color.rgb(24, 30, 44))
                setTypeface(typeface, Typeface.BOLD)
            })
            addView(TextView(this@MainActivity).apply {
                text = "Your phone, available on Linux"
                textSize = 16f
                setTextColor(Color.rgb(92, 99, 116))
                setPadding(0, dp(2), 0, dp(16))
            })
            addView(panel(sectionTitle("Status"), status, homeConnect), LinearLayout.LayoutParams(-1, -2))
            listOf(
                menuButton("Connection", "Pair, connect, or troubleshoot discovery") { showPage(connectionPage) },
                menuButton("Permissions", "Notifications, calls, network, and background access") { showPage(permissionsPage) },
                menuButton("Activity", "Notification and media service status") { showPage(activityPage) },
                menuButton("Presentation", "Control slides and the pointer") { showPage(presentationPage) },
                menuButton("System volume", "Control Linux audio output") { showPage(volumePage) },
                menuButton("Contacts", "Send an on-demand contacts snapshot") { showPage(contactsPage) },
                menuButton("Updates", "Install a verified update received from Linux") { showPage(updatesPage) },
                menuButton("Diagnostics", "Test notifications and media controls") { showPage(diagnosticsPage) },
            ).forEach { item ->
                addView(item, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(12) })
            }
        }
        homePage = home
        pageHost = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(20), dp(20), dp(32))
        }
        showPage(home)

        @Suppress("DEPRECATION")
        window.decorView.systemUiVisibility = View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR or
            View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR
        window.statusBarColor = Color.rgb(238, 241, 247)
        window.navigationBarColor = Color.rgb(238, 241, 247)
        if (android.os.Build.VERSION.SDK_INT >= 30) {
            window.setDecorFitsSystemWindows(true)
        }
        setContentView(ScrollView(this).apply {
            fitsSystemWindows = true
            clipToPadding = false
            setBackgroundColor(Color.rgb(238, 241, 247))
            addView(pageHost)
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
                startForegroundService(service)
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
        private const val CONTACTS_PERMISSION_REQUEST = 46
    }
}
