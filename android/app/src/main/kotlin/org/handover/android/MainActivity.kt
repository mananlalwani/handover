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
    private val pickFileRequest = 47
    private lateinit var status: TextView
    private lateinit var notificationStatus: TextView
    private lateinit var mediaStatus: TextView
    private lateinit var awakeStatus: TextView
    private lateinit var awakeSwitch: Switch
    private lateinit var laptopAudioStatus: TextView
    private lateinit var laptopAudioControls: LinearLayout
    private val laptopAudioProfileButtons = mutableMapOf<String, Button>()
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
        if (::laptopAudioStatus.isInitialized) refreshLaptopAudioStatus()
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

    private fun refreshLaptopAudioStatus() {
        if (::laptopAudioControls.isInitialized) refreshLaptopAudioControls()
        val audio = HandoverForegroundService.laptopAudioStatus()
        if (audio == null) {
            laptopAudioStatus.text = "Status unavailable. Connect to your laptop to read stream state.\n\nAudible delay, including Bluetooth, is not measured."
            laptopAudioProfileButtons.values.forEach { it.backgroundTintList = android.content.res.ColorStateList.valueOf(Color.rgb(238, 240, 246)) }
            return
        }
        val state = audio.optString("state").takeIf { it.isNotBlank() } ?: "unavailable"
        val profile = audio.optString("profile").takeIf { it.isNotBlank() } ?: "unavailable"
        val enabled = if (audio.has("enabled") && !audio.isNull("enabled")) audio.optBoolean("enabled").toString() else "unavailable"
        val failure = audio.optString("failure").takeIf { !audio.isNull("failure") && it.isNotBlank() }
        val retry = if (audio.has("retry_attempt") && !audio.isNull("retry_attempt")) audio.optInt("retry_attempt").toString() else "unavailable"
        val diagnostics = audio.optJSONObject("diagnostics")
        val sender = diagnostics?.optJSONObject("sender")
        val receiver = diagnostics?.optJSONObject("receiver")
        fun metric(obj: org.json.JSONObject?, key: String, suffix: String = ""): String =
            if (obj == null || !obj.has(key) || obj.isNull(key)) "unavailable" else "${obj.get(key)}$suffix"
        fun age(obj: org.json.JSONObject?): String {
            if (obj == null || !obj.has("updated_at_ms") || obj.isNull("updated_at_ms")) return "unavailable"
            val elapsed = (System.currentTimeMillis() - obj.optLong("updated_at_ms")).coerceAtLeast(0L)
            return when {
                elapsed < 1_000 -> "just now"
                elapsed < 60_000 -> "${elapsed / 1_000}s ago"
                else -> "${elapsed / 60_000}m ago"
            }
        }
        laptopAudioStatus.text = buildString {
            append("Confirmed by laptop\n")
            val requestNotice = audio.optString("request_notice")
            if (requestNotice.isNotBlank()) append("$requestNotice\n\n")
            append("State: ${state.replace('_', ' ')}\nRequested streaming: $enabled\nConfirmed preset: $profile\nReconnect attempts: $retry")
            if (failure != null) append("\nReason: " + when (failure) {
                "disconnected" -> "The connection was interrupted."
                "connection_failed" -> "The audio connection failed."
                "capture_failed" -> "The laptop could not capture audio. Check its audio service."
                "startup_timeout" -> "The phone did not start audio in time."
                "audio_focus" -> "Another app or call is using phone audio."
                "receiver_failed" -> "The phone audio output stopped or failed."
                "retries_exhausted" -> "Automatic retries stopped. Tap Start stream to try again."
                else -> failure.replace('_', ' ')
            })
            append("\n\nSender diagnostics (sample ${age(sender)})\n")
            append("Encoder bitrate: ${metric(sender, "encoder_bitrate_bps", " bps")}\n")
            append("Packet duration: ${metric(sender, "packet_duration_ms", " ms")}\n")
            append("Capture drops: ${metric(sender, "capture_dropped_packets")}")
            append("\n\nReceiver diagnostics (sample ${age(receiver)})\n")
            append("Route: ${metric(receiver, "route")}\nTransport: ${metric(receiver, "transport")}\n")
            append("Round trip: ${metric(receiver, "round_trip_ms", " ms")}\nJitter: ${metric(receiver, "jitter_ms", " ms")}\n")
            append("Average jitter buffer: ${metric(receiver, "jitter_buffer_ms", " ms")}\nBitrate: ${metric(receiver, "bitrate_bps", " bps")}\n")
            append("Packets received: ${metric(receiver, "packets_received")}\nPackets lost: ${metric(receiver, "packets_lost")}")
            append("\n\nAudible delay, including Bluetooth, is not measured.")
        }
        laptopAudioProfileButtons.forEach { (key, button) ->
            button.backgroundTintList = android.content.res.ColorStateList.valueOf(
                if (key == profile) Color.rgb(205, 218, 255) else Color.rgb(238, 240, 246),
            )
        }
    }

    private fun refreshLaptopAudioControls() {
        val host = laptopAudioControls
        host.removeAllViews()
        val snapshot = HandoverForegroundService.laptopAudioControls()
        if (snapshot == null) {
            host.addView(audioNote("Routing controls unavailable. Connect to an updated laptop daemon to read confirmed choices."))
            host.addView(secondary(Button(this).apply {
                text = "Refresh apps / players"
                setOnClickListener {
                    val queued = HandoverForegroundService.requestLaptopAudioControls()
                    android.widget.Toast.makeText(this@MainActivity,
                        if (queued) "Refresh request queued for laptop" else "Laptop routing unavailable. Update the desktop daemon or reconnect.",
                        android.widget.Toast.LENGTH_SHORT).show()
                }
            }))
            return
        }
        fun patch(change: org.json.JSONObject, label: String) {
            val queued = HandoverForegroundService.configureLaptopAudio(change)
            android.widget.Toast.makeText(this,
                if (queued) "$label request queued for laptop" else "Connect to your laptop first",
                android.widget.Toast.LENGTH_SHORT).show()
        }
        host.addView(audioNote("These choices apply to every receiving phone."))
        val source = snapshot.optJSONObject("settings")?.optJSONObject("source")
        val settingsAvailable = snapshot.optBoolean("settings_available", false)
        val sourceKind = source?.optString("kind")
        val selectedIds = source?.optJSONArray("ids") ?: org.json.JSONArray()
        val selected = (0 until selectedIds.length()).mapNotNull { selectedIds.optString(it).takeIf(String::isNotBlank) }.toSet()
        val applications = snapshot.optJSONArray("applications") ?: org.json.JSONArray()
        val availableApps = (0 until applications.length()).mapNotNull { applications.optJSONObject(it) }
        val selectedAppNames = availableApps.filter { it.optString("id") in selected }.map { it.optString("name", it.optString("id")) }
        val sourceSummary = if (!settingsAvailable || sourceKind == null) "unavailable"
            else if (sourceKind == "all") "All apps" else if (selected.isEmpty()) "No apps selected (audio will be silent)"
            else selectedAppNames.plus((selected - availableApps.map { it.optString("id") }.toSet()).map { "Unavailable app ($it)" }).joinToString(", ")
        host.addView(audioNote("Source: $sourceSummary"))
        val applicationsAvailable = snapshot.optBoolean("applications_available", false)
        val playersAvailable = snapshot.optBoolean("players_available", false)
        host.addView(secondary(Button(this).apply {
            text = "Choose source apps"
            isEnabled = true
            setOnClickListener {
                val rows = mutableListOf("All apps")
                val options = mutableListOf<Pair<String, String>>()
                availableApps.filter { applicationsAvailable || it.optString("id") in selected }.forEach { app ->
                    options.add(app.optString("id") to app.optString("name", app.optString("id")))
                }
                val unavailable = selected - options.map { it.first }.toSet()
                unavailable.forEach { options.add(it to "Unavailable app ($it)") }
                rows.addAll(options.map { option ->
                    val app = availableApps.firstOrNull { it.optString("id") == option.first }
                    option.second + if (app != null && !app.optBoolean("available", true)) " (unavailable)" else ""
                })
                val checked = BooleanArray(rows.size) { index -> if (index == 0) sourceKind == "all" else options[index - 1].first in selected }
                AlertDialog.Builder(this@MainActivity)
                    .setTitle("Audio source")
                    .setMultiChoiceItems(rows.toTypedArray(), checked) { dialog, which, isChecked ->
                        checked[which] = isChecked
                        val list = (dialog as AlertDialog).listView
                        if (which == 0 && isChecked) {
                            for (i in 1 until checked.size) { checked[i] = false; list.setItemChecked(i, false) }
                        } else if (which > 0 && isChecked) {
                            checked[0] = false; list.setItemChecked(0, false)
                        }
                    }
                    .setPositiveButton("Apply") { _, _ ->
                        if (checked[0]) patch(org.json.JSONObject().put("source", org.json.JSONObject().put("kind", "all")), "Source")
                        else {
                            val ids = org.json.JSONArray()
                            options.forEachIndexed { index, pair -> if (checked[index + 1]) ids.put(pair.first) }
                            patch(org.json.JSONObject().put("source", org.json.JSONObject().put("kind", "applications").put("ids", ids)), "Source")
                        }
                    }.setNegativeButton("Cancel", null).show()
            }
        }))
        host.addView(audioNote(if (applicationsAvailable) "${availableApps.count { it.optBoolean("available", true) }} apps observed" else "App list unavailable"))

        val settings = snapshot.optJSONObject("settings")
        val output = settings?.optString("output") ?: ""
        host.addView(audioNote("Output: ${when (output) { "phone" -> "Phone only"; "laptop" -> "Laptop only"; "both" -> "Both"; else -> "unavailable" }}"))
        host.addView(LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            listOf("Phone only" to "phone", "Laptop only" to "laptop", "Both" to "both").forEach { (label, value) ->
                addView(secondary(Button(this@MainActivity).apply {
                    text = label
                    isEnabled = true
                    backgroundTintList = android.content.res.ColorStateList.valueOf(if (output == value) Color.rgb(205, 218, 255) else Color.rgb(238, 240, 246))
                    setOnClickListener { patch(org.json.JSONObject().put("output", value), "Output") }
                }), LinearLayout.LayoutParams(0, -2, 1f))
            }
        })
        host.addView(audioNote("Laptop only plays audio locally and sends silence to the phone while keeping the stream connected. Both plays on the laptop and phone without synchronization, so an echo may occur."))

        val playback = settings?.optJSONObject("playback")
        val playbackKind = playback?.optString("kind")
        val playerId = playback?.optString("id") ?: ""
        val players = snapshot.optJSONArray("players") ?: org.json.JSONArray()
        val playerItems = (0 until players.length()).mapNotNull { players.optJSONObject(it) }
        val selectedPlayerName = playerItems.firstOrNull { it.optString("id") == playerId }?.optString("name") ?: "Unavailable player"
        host.addView(audioNote("Player: ${if (!settingsAvailable || playbackKind == null) "unavailable" else if (playbackKind == "automatic") "Automatic" else selectedPlayerName}"))
        host.addView(secondary(Button(this).apply {
            text = "Choose player"
            isEnabled = true
            setOnClickListener {
                val choices = mutableListOf("Automatic")
                val ids = mutableListOf<String?>(null)
                if (playersAvailable) playerItems.forEach { choices.add(it.optString("name", it.optString("id"))); ids.add(it.optString("id")) }
                val selectedIndex = if (playbackKind == "automatic") 0 else ids.indexOf(playerId)
                AlertDialog.Builder(this@MainActivity).setTitle("Laptop player")
                    .setSingleChoiceItems(choices.toTypedArray(), selectedIndex) { dialog, which ->
                        val selectedId = ids[which]
                        val value = if (selectedId == null) org.json.JSONObject().put("kind", "automatic")
                            else org.json.JSONObject().put("kind", "player").put("id", selectedId)
                        patch(org.json.JSONObject().put("playback", value), "Player")
                        dialog.dismiss()
                    }.setNegativeButton("Cancel", null).show()
            }
        }))
        host.addView(audioNote(if (playersAvailable) "${playerItems.size} players observed" else "Player list unavailable"))
        host.addView(secondary(Button(this).apply {
            text = "Refresh apps / players"
            setOnClickListener {
                val queued = HandoverForegroundService.requestLaptopAudioControls()
                android.widget.Toast.makeText(this@MainActivity,
                    if (queued) "Refresh request queued for laptop" else "Connect to your laptop first",
                    android.widget.Toast.LENGTH_SHORT).show()
            }
        }))
        if (!settingsAvailable) host.addView(audioNote("Confirmed routing settings unavailable"))
    }

    private fun audioNote(message: String) = TextView(this).apply {
        text = message
        textSize = 14f
        setTextColor(Color.rgb(70, 77, 94))
        setPadding(0, dp(4), 0, dp(4))
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
            if (intent.action == NativeTransport.ACTION_AUDIO_STATUS) {
                refreshLaptopAudioStatus()
                return
            }
            if (intent.action == NativeTransport.ACTION_AUDIO_CONTROLS) {
                val payload = runCatching {
                    org.json.JSONObject(intent.getStringExtra(NativeTransport.EXTRA_JSON).orEmpty())
                }.getOrNull()
                val failure = payload?.optString("failure")?.takeIf { it.isNotBlank() }
                if (failure != null) {
                    val message = when (failure) {
                        "unavailable" -> "Laptop audio routing is unavailable"
                        "unknown_application" -> "That app is no longer available"
                        "unknown_player" -> "That player is no longer available"
                        "invalid_selection" -> "The laptop rejected that routing choice"
                        else -> "The laptop could not apply that routing choice"
                    }
                    android.widget.Toast.makeText(this@MainActivity, message, android.widget.Toast.LENGTH_LONG).show()
                }
                refreshLaptopAudioStatus()
                return
            }
            if (intent.action == HandoverForegroundService.ACTION_SHARE_QUEUE_FAILED) {
                android.widget.Toast.makeText(this@MainActivity,
                    "Share could not be queued. Check the connection, link, file name, and 100 MiB limit.",
                    android.widget.Toast.LENGTH_LONG).show()
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
            addAction(NativeTransport.ACTION_AUDIO_STATUS)
            addAction(NativeTransport.ACTION_AUDIO_CONTROLS)
            addAction(HandoverForegroundService.ACTION_SHARE_QUEUE_FAILED)
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
        if (savedInstanceState == null) handleShareIntent(intent)
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
            panel(sectionTitle("Notifications"), notificationAccess, appNotificationAccess,
                secondary(Button(this).apply {
                    text = "Choose apps to forward"
                    setOnClickListener { openNotificationForwarding() }
                })),
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
        val laptopAudioPage = featurePage(
            "Laptop audio", "Control the audio stream and the laptop player separately.",
            panel(sectionTitle("Laptop status"), TextView(this).apply {
                laptopAudioStatus = this
                textSize = 14f
                setTextColor(Color.rgb(70, 77, 94))
                text = "Reading laptop audio status…"
            }),
            panel(sectionTitle("Audio routing"), LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                laptopAudioControls = this
            }),
            panel(sectionTitle("Stream to this phone"), TextView(this).apply {
                text = "Pausing the stream leaves the laptop player running. Laptop audio output returns when the last stream stops."
                textSize = 14f
                setTextColor(Color.rgb(70, 77, 94))
            }, LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                listOf("Start stream" to true, "Pause stream" to false).forEach { (label, enabled) ->
                    addView(secondary(Button(this@MainActivity).apply {
                        text = label
                        setOnClickListener {
                            val queued = HandoverForegroundService.setLaptopAudioStreaming(enabled)
                            android.widget.Toast.makeText(this@MainActivity,
                                if (queued) "Stream request queued for laptop"
                                else "Connect to your laptop first",
                                android.widget.Toast.LENGTH_SHORT).show()
                        }
                    }), LinearLayout.LayoutParams(0, -2, 1f))
                }
            }),
            panel(sectionTitle("Streaming preset"), TextView(this).apply {
                text = "Choose a preset. The confirmed preset changes after the laptop reports it."
                textSize = 14f
                setTextColor(Color.rgb(70, 77, 94))
            }, LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                listOf("Music" to "music", "Balanced" to "balanced", "Low delay" to "low_delay").forEach { (label, value) ->
                    val button = secondary(Button(this@MainActivity).apply {
                        text = label
                        setOnClickListener {
                            val queued = HandoverForegroundService.setLaptopAudioProfile(value)
                            android.widget.Toast.makeText(this@MainActivity,
                                if (queued) "Preset request queued for laptop" else "Connect to your laptop first",
                                android.widget.Toast.LENGTH_SHORT).show()
                        }
                    })
                    laptopAudioProfileButtons[value] = button
                    addView(button, LinearLayout.LayoutParams(0, -2, 1f))
                }
            }),
            panel(sectionTitle("Playback"), primary(Button(this).apply {
                text = "Pause / resume"
                setOnClickListener {
                    val queued = HandoverForegroundService.toggleLaptopPlayback()
                    android.widget.Toast.makeText(this@MainActivity,
                        if (queued) "Playback request queued for laptop"
                        else "Connect to your laptop and start audio streaming first",
                        android.widget.Toast.LENGTH_SHORT).show()
                }
            })),
        )
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
                    text = "Choose a file here or share files and links from another app. Send clipboard from the tile or below. Slides, the Linux pointer, volume, files, and allowlisted commands are on the following pages."
                    setTextColor(Color.rgb(70, 77, 94))
                },
                primary(Button(this@MainActivity).apply {
                    text = "Send file to desktop"
                    setOnClickListener {
                        if (!canSendToDesktop()) return@setOnClickListener
                        startActivityForResult(Intent(Intent.ACTION_GET_CONTENT).apply {
                            type = "*/*"
                            addCategory(Intent.CATEGORY_OPENABLE)
                        }, pickFileRequest)
                    }
                }),
                secondary(Button(this@MainActivity).apply {
                    text = "Send current clipboard to Linux"
                    setOnClickListener { HandoverForegroundService.sendClipboard() }
                }),
            ), LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(12) })
            listOf(
                menuButton("Connection", "Pair, connect, or troubleshoot discovery") { showPage(connectionPage) },
                menuButton("Permissions", "Notifications, calls, network, and background access") { showPage(permissionsPage) },
                menuButton("Notifications", "Choose which apps notify your laptop") { openNotificationForwarding() },
                menuButton("Activity", "Notification and media service status") { showPage(activityPage) },
                menuButton("Presentation", "Control slides and the pointer") { showPage(presentationPage) },
                menuButton("Remote input", "Control the Linux pointer and keyboard") { showPage(remoteInputPage) },
                menuButton("Remote desktop", "Browse files and run configured commands") { showPage(remotePage) },
                menuButton("Laptop audio", "Pause or resume playback on Linux") { showPage(laptopAudioPage) },
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

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != pickFileRequest || resultCode != RESULT_OK) return
        val uri = data?.data ?: return
        sendFileWithConfirmation(uri)
    }

    private fun canSendToDesktop(): Boolean {
        val paired = NativeTransport.trustedPeerFingerprint(this) != null
        if (paired && HandoverForegroundService.connectionState() == "connected") return true
        android.widget.Toast.makeText(this,
            if (paired) "Connect to the desktop before sending" else "Pair a desktop before sending",
            android.widget.Toast.LENGTH_LONG).show()
        return false
    }

    private fun sendFileWithConfirmation(uri: Uri) {
        confirmShare(DesktopShare.Files(listOf(uri)))
    }

    private fun confirmShare(share: DesktopShare) {
        if (!canSendToDesktop()) return
        val target = NativeTransport.trustedPeerFingerprint(this) ?: return
        val description = when (share) {
            is DesktopShare.Files -> if (share.uris.size == 1) "Send this file" else "Send ${share.uris.size} files"
            is DesktopShare.Url -> "Send this link"
        }
        AlertDialog.Builder(this)
            .setTitle(R.string.send_to_pc)
            .setMessage("$description to paired desktop $target?")
            .setNegativeButton("Cancel", null)
            .setPositiveButton("Send") { _, _ ->
                if (!canSendToDesktop()) return@setPositiveButton
                if (NativeTransport.trustedPeerFingerprint(this) != target) {
                    android.widget.Toast.makeText(this,
                        "Paired desktop changed. Share again to confirm the destination.",
                        android.widget.Toast.LENGTH_LONG).show()
                    return@setPositiveButton
                }
                // Each file keeps its own transfer ID and eventual result.
                // Pass the URI grant to the service for asynchronous streaming.
                share.serviceIntents(this).forEach { service ->
                    startForegroundService(service)
                }
            }.show()
    }

    private fun handleShareIntent(intent: Intent) {
        if (intent.action != Intent.ACTION_SEND && intent.action != Intent.ACTION_SEND_MULTIPLE) return
        val share = DesktopShare.fromIntent(intent)
        if (share == null) {
            android.widget.Toast.makeText(this,
                "Share a web link or up to 32 files. Each file must be no larger than 100 MiB.",
                android.widget.Toast.LENGTH_LONG).show()
            return
        }
        confirmShare(share)
    }

    companion object {
        internal const val LOCAL_NETWORK_REQUEST = 42
        internal const val POST_NOTIFICATIONS_REQUEST = 43
        internal const val CALL_PERMISSIONS_REQUEST = 45
        internal const val CONTACTS_PERMISSION_REQUEST = 46
    }
}
