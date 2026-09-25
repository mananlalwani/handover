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
    private lateinit var awakeStatus: TextView
    private lateinit var awakeSwitch: Switch
    private lateinit var transferStatus: LinearLayout
    internal lateinit var capabilitiesStatus: TextView
    internal lateinit var remoteResult: TextView
    private lateinit var updateStatus: TextView
    private lateinit var homeConnect: Button
    internal lateinit var pageHost: LinearLayout
    internal var homePage: View? = null
    private var shownPairCode: String? = null
    private var pairDialog: AlertDialog? = null
    private var testCounter = 1
    private var reconnectPromptShown = false
    private var connectionState = "offline"
    private var githubUpdateStatus = "Check GitHub for a signed release APK"
    internal val permissionButtons = mutableListOf<Pair<Button, () -> Boolean>>()


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

    internal fun deviceAdminComponent() = ComponentName(this, HandoverDeviceAdminReceiver::class.java)

    internal fun deviceAdminEnabled(): Boolean =
        getSystemService(android.app.admin.DevicePolicyManager::class.java)
            .isAdminActive(deviceAdminComponent())

    internal fun refreshStatus() {
        connectionState = HandoverForegroundService.connectionState()
        val peer = NativeTransport.trustedPeerFingerprint(this) ?: "No paired desktop"
        status.text = "Connection: ${connectionState.replaceFirstChar { it.uppercase() }}\n\nDevice identity: ${DeviceIdentityStore(this).deviceId}\n\nPaired desktop: $peer"
        val listener = if (HandoverNotificationService.isEnabled(this)) "granted" else "not granted"
        val reply = TestNotificationReceiver.lastReply(this)?.let { "\nLast test reply: $it" }.orEmpty()
        notificationStatus.text = "Notification access: $listener$reply"
        mediaStatus.text = if (TestMediaSession.isActive()) "Test media: playing" else "Test media: stopped"
        awakeStatus.text = "Desktop requested: ${HandoverForegroundService.desktopAwakeRequested()}\n" +
            "Phone held awake: ${HandoverForegroundService.phoneAwakeHeld()}"
        if (::awakeSwitch.isInitialized) {
            val requested = HandoverForegroundService.desktopAwakeRequested()
            if (awakeSwitch.isChecked != requested) {
                awakeSwitch.setOnCheckedChangeListener(null)
                awakeSwitch.isChecked = requested
                awakeSwitch.setOnCheckedChangeListener { _, enabled ->
                    HandoverForegroundService.requestDesktopAwake(enabled)
                    refreshStatus()
                }
            }
        }
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
            append("\nRemote lock: ${if (deviceAdminEnabled()) "granted" else "not granted"}")
        }
        val installed = packageManager.getPackageInfo(packageName, 0)
        val pending = AppUpdater.pending(this)
        updateStatus.text = if (pending == null) {
            "Installed: ${installed.versionName}\nNo verified update ready\n$githubUpdateStatus"
        } else {
            "Installed: ${installed.versionName}\nReady to install: ${pending.versionName.ifEmpty { pending.versionCode.toString() }}\n$githubUpdateStatus"
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

    private fun checkGitHubUpdates(button: Button) {
        button.isEnabled = false
        githubUpdateStatus = "Checking GitHub..."
        refreshStatus()
        Thread {
            val result = runCatching { GitHubUpdates.checkAndDownload(this) }
            runOnUiThread {
                button.isEnabled = true
                githubUpdateStatus = result.fold(
                    onSuccess = { outcome -> when (outcome) {
                        GitHubUpdateResult.Current -> "GitHub release is up to date"
                        is GitHubUpdateResult.Ready ->
                            "GitHub APK verified: ${outcome.update.versionName}. Tap Install downloaded update."
                        is GitHubUpdateResult.NoSignedApk ->
                            "Release ${outcome.version} has no signed Android APK"
                        GitHubUpdateResult.RejectedApk ->
                            "APK rejected: package, version, or signing key differs. Older debug builds require a one-time reinstall."
                    } },
                    onFailure = { error -> "GitHub check failed: ${error.message ?: "try again later"}" },
                )
                refreshStatus()
            }
        }.start()
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
            if (intent.action == NativeTransport.ACTION_REMOTE_RESULT) {
                showRemoteResult(intent.getStringExtra(NativeTransport.EXTRA_JSON).orEmpty())
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

    private fun showRemoteResult(payload: String) {
        if (!::remoteResult.isInitialized) return
        val message = runCatching { org.json.JSONObject(payload) }.getOrNull() ?: return
        remoteResult.text = when (message.optString("type")) {
            "filesystem_entries" -> {
                val path = message.optString("path", ".")
                val entries = message.optJSONArray("entries")
                buildString {
                    append("Listing $path")
                    if (entries == null || entries.length() == 0) {
                        append("\n(empty)")
                    } else {
                        for (index in 0 until entries.length()) {
                            val entry = entries.optJSONObject(index) ?: continue
                            val name = entry.optString("name")
                            if (name.isEmpty()) continue
                            append('\n')
                            append(if (entry.optBoolean("directory")) "dir  " else "file ")
                            append(name)
                        }
                    }
                }
            }
            "filesystem_failure" -> "Directory listing failed"
            "custom_command_list" -> {
                val names = message.optJSONArray("names")
                buildString {
                    append("Allowlisted commands")
                    if (names == null || names.length() == 0) {
                        append("\n(none configured)")
                    } else {
                        for (index in 0 until names.length()) {
                            append('\n')
                            append(names.optString(index))
                        }
                    }
                }
            }
            "custom_command_result" -> {
                val accepted = message.optBoolean("accepted")
                val failure = message.optString("failure").takeIf { it.isNotEmpty() }
                if (accepted) {
                    "Command accepted (exit ${message.optInt("exit_code", 0)})"
                } else {
                    "Command rejected${failure?.let { " ($it)" } ?: ""}"
                }
            }
            else -> "Remote result received"
        }
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
            addAction(NativeTransport.ACTION_REMOTE_RESULT)
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
        HandoverForegroundService.startIfPaired(this)
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
        awakeStatus = TextView(this).apply {
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
        val deviceAdminAccess = Button(this).apply {
            text = "Enable remote lock"
            setOnClickListener {
                if (deviceAdminEnabled()) {
                    refreshStatus()
                    return@setOnClickListener
                }
                startActivity(Intent(android.app.admin.DevicePolicyManager.ACTION_ADD_DEVICE_ADMIN)
                    .putExtra(android.app.admin.DevicePolicyManager.EXTRA_DEVICE_ADMIN, deviceAdminComponent())
                    .putExtra(android.app.admin.DevicePolicyManager.EXTRA_ADD_EXPLANATION,
                        "Handover uses this permission only to lock the phone from a trusted paired desktop."))
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
            backgroundAccess, callAccess, deviceAdminAccess, postTest, updateTest, removeTest, startTestMedia,
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
        permissionButtons += deviceAdminAccess to { deviceAdminEnabled() }
        permissionButtons += localNetworkAccess to {
            android.os.Build.VERSION.SDK_INT < 37 ||
                checkSelfPermission("android.permission.ACCESS_LOCAL_NETWORK") == PackageManager.PERMISSION_GRANTED
        }
        permissionButtons += batteryAccess to {
            getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(packageName)
        }
        refreshPermissionButtons()

        val connectionPage = featurePage(
            "Connection", "Connect this phone to your trusted Linux desktop.",
            panel(sectionTitle("Handover service"), start),
            panel(sectionTitle("Manual connection"), address, manualConnect),
            panel(sectionTitle("Pairing"), revoke),
        )
        val permissionsPage = featurePage(
            "Permissions", "Enable only the capabilities you want Handover to provide.",
            panel(sectionTitle("Current access"), capabilitiesStatus),
            panel(sectionTitle("Notifications"), notificationAccess, appNotificationAccess),
            panel(sectionTitle("Calls"), callAccess),
            panel(sectionTitle("Device actions"), deviceAdminAccess),
            panel(sectionTitle("Connectivity & background"), localNetworkAccess,
                batteryAccess, backgroundAccess),
            panel(sectionTitle("Clipboard"), Switch(this).apply {
                text = "Send clipboard changes while Handover is visible"
                isChecked = HandoverForegroundService.clipboardSyncEnabled()
                setOnCheckedChangeListener { _, enabled ->
                    HandoverForegroundService.setClipboardSync(enabled)
                }
            }, TextView(this).apply {
                text = "For manual background sends, add the Handover 'Send clipboard' tile from the Quick Settings tile editor."
                textSize = 14f
                setTextColor(Color.rgb(92, 99, 116))
            }),
            panel(sectionTitle("Advanced automatic clipboard sync"), TextView(this).apply {
                text = "Android blocks background clipboard access. This optional workaround reads only ClipboardService log events for Handover, then opens a transient 1x1 activity for one clipboard read. It requires an ADB READ_LOGS grant."
                textSize = 14f
                setTextColor(Color.rgb(92, 99, 116))
            }, Switch(this).apply {
                text = "Background reads via transient activity (needs display permission and ADB READ_LOGS grant)"
                isChecked = HandoverForegroundService.overlayAssistEnabled()
                setOnCheckedChangeListener { _, enabled ->
                    if (enabled && !HandoverForegroundService.overlayPermissionGranted()) {
                        android.widget.Toast.makeText(this@MainActivity,
                            "Allow display over other apps first", android.widget.Toast.LENGTH_LONG).show()
                        isChecked = false
                        return@setOnCheckedChangeListener
                    }
                    HandoverForegroundService.setOverlayAssist(enabled)
                }
            }, secondary(Button(this).apply {
                text = "Allow display over other apps"
                setOnClickListener {
                    startActivity(Intent(android.provider.Settings.ACTION_MANAGE_OVERLAY_PERMISSION,
                        Uri.parse("package:$packageName")))
                }
            })),
        )
        val activityPage = featurePage(
            "Activity", "Current phone-side Handover activity.",
            panel(sectionTitle("Notifications"), notificationStatus),
            panel(sectionTitle("Media"), mediaStatus),
            panel(sectionTitle("Stay awake"), awakeStatus, Switch(this).apply {
                text = "Ask the desktop to stay awake"
                isChecked = HandoverForegroundService.desktopAwakeRequested()
                setOnCheckedChangeListener { _, enabled ->
                    HandoverForegroundService.requestDesktopAwake(enabled)
                    refreshStatus()
                }
            }.also { awakeSwitch = it }),
            panel(sectionTitle("Recent transfers"), transferStatus,
                secondary(Button(this).apply {
                    text = "Clear all"
                    setOnClickListener { TransferHistory.clear(this@MainActivity); refreshStatus() }
                })),
        )
        val presentationPage = buildPresentationPage()
        val diagnosticsPage = featurePage(
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
            text = "Scan received APKs"
            setOnClickListener { scanForUpdates() }
        })
        val checkGitHub = secondary(Button(this).apply {
            text = "Check GitHub releases"
            setOnClickListener { checkGitHubUpdates(this) }
        })
        val updatesPage = featurePage(
            "Updates", "Download a signed GitHub release or install an APK received from your desktop.",
            panel(sectionTitle("App version"), updateStatus, checkGitHub, scanUpdates, installUpdate),
            panel(sectionTitle("Security"), TextView(this).apply {
                text = "Only a newer Handover APK signed by the same certificate is accepted. Android always asks before installing it."
                textSize = 14f
                setTextColor(Color.rgb(70, 77, 94))
            }),
        )
        val remoteInputPage = buildRemoteInputPage()
        val remotePage = buildRemoteDesktopPage()
        val volumePage = featurePage(
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
        val contactsPage = featurePage(
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
            addView(panel(
                sectionTitle("On this phone"),
                TextView(this@MainActivity).apply {
                    text = "Share files and links from any app with Handover. Send clipboard from the tile or below. Slides, the Linux pointer, volume, files, and allowlisted commands are on the following pages."
                    setTextColor(Color.rgb(70, 77, 94))
                },
                secondary(Button(this@MainActivity).apply {
                    text = "Send current clipboard to Linux"
                    setOnClickListener { HandoverForegroundService.sendClipboard() }
                }),
            ), LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(12) })
            listOf(
                menuButton("Connection", "Pair, connect, or troubleshoot discovery") { showPage(connectionPage) },
                menuButton("Permissions", "Notifications, calls, network, and background access") { showPage(permissionsPage) },
                menuButton("Activity", "Notification and media service status") { showPage(activityPage) },
                menuButton("Presentation", "Control slides and the pointer") { showPage(presentationPage) },
                menuButton("Remote input", "Control the Linux pointer and keyboard") { showPage(remoteInputPage) },
                menuButton("Remote desktop", "Browse files and run configured commands") { showPage(remotePage) },
                menuButton("System volume", "Control Linux audio output") { showPage(volumePage) },
                menuButton("Contacts", "Send an on-demand contacts snapshot") { showPage(contactsPage) },
                menuButton("Updates", "Check GitHub or install a verified APK") { showPage(updatesPage) },
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
        internal const val LOCAL_NETWORK_REQUEST = 42
        internal const val POST_NOTIFICATIONS_REQUEST = 43
        internal const val CALL_PERMISSIONS_REQUEST = 45
        internal const val CONTACTS_PERMISSION_REQUEST = 46
    }
}
