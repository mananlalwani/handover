package org.handover.android

import android.annotation.SuppressLint
import android.app.Notification
import android.app.RemoteInput
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.provider.Settings
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log
import org.json.JSONObject

/** Phone-side notification state for the native backend.
 *
 * Posted/removed events come from the platform [NotificationListenerService];
 * dismissal, action, and reply requests arrive over the native transport after
 * the desktop is paired. Normal clients never see which backend produced a
 * notification: the daemon normalizes these into the shared Handover model.
 */
data class WireNotificationAction(val id: String, val label: String)

data class WireNotification(
    val key: String,
    val app: String,
    val title: String,
    val body: String,
    val clearable: Boolean,
    val actions: List<WireNotificationAction>,
    val replySupported: Boolean,
)

class HandoverNotificationService : NotificationListenerService() {
    override fun onListenerConnected() {
        instance = this
        Log.i(TAG, "Notification listener connected")
        pushSync()
    }

    override fun onListenerDisconnected() {
        if (instance === this) instance = null
        Log.i(TAG, "Notification listener disconnected")
        // Keep daemon state on transient disconnects; a reconnect resyncs.
        // When the permission itself was revoked, report disabled so the
        // daemon clears stale entries instead of showing ghosts.
        if (!isEnabled(this)) transport?.syncNotifications(false, emptyList())
    }

    override fun onNotificationPosted(sbn: StatusBarNotification) {
        snapshot(this, sbn)?.let { transport?.publishNotification(it) }
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification) {
        val key = sbn.key ?: return
        if (isValidKey(key)) transport?.retractNotification(key)
    }

    /** Push the full current list; called on connect and on transport request. */
    fun pushSync() {
        val (enabled, list) = snapshotFor(this)
        transport?.syncNotifications(enabled, list)
    }

    companion object {
        private const val TAG = "HandoverNotifications"
        // Same-process service ownership: the foreground service sets this on
        // create and clears it on destroy. Suppressed because the transport
        // holds the application context, not an activity.
        @SuppressLint("StaticFieldLeak")
        @Volatile var transport: NativeTransport? = null
        @Volatile private var instance: HandoverNotificationService? = null

        const val MAX_KEY = 256
        const val MAX_APP = 128
        const val MAX_TITLE = 512
        const val MAX_BODY = 4096
        const val MAX_ACTIONS = 8
        const val MAX_ACTION_ID = 64
        const val MAX_ACTION_LABEL = 128
        const val MAX_REPLY_TEXT = 1024

        /** Whether the user granted notification-listener access to this app. */
        fun isEnabled(context: Context): Boolean {
            val flat = Settings.Secure.getString(
                context.contentResolver, "enabled_notification_listeners",
            ) ?: return false
            val me = ComponentName(context, HandoverNotificationService::class.java)
            return flat.split(':').any {
                runCatching {
                    ComponentName.unflattenFromString(it) == me
                }.getOrDefault(false)
            }
        }

        fun isValidKey(key: String): Boolean =
            key.isNotEmpty() && key.length <= MAX_KEY && key.none { it.isISOControl() }

        fun truncateTo(value: String, max: Int): String =
            if (value.length <= max) value else value.substring(0, max)

        /** Current permission state plus the active notification list. */
        fun snapshotFor(context: Context): Pair<Boolean, List<WireNotification>> {
            if (!isEnabled(context)) return false to emptyList()
            val svc = instance ?: return true to emptyList()
            val list = runCatching { svc.activeNotifications }.getOrNull()
                ?.mapNotNull { snapshot(svc, it) } ?: emptyList()
            // Never log titles or bodies; counts only.
            Log.i(TAG, "Notification sync: ${list.size} active")
            return true to list
        }

        /** Map one platform notification into the wire model. Null when unusable. */
        fun snapshot(context: Context, sbn: StatusBarNotification): WireNotification? {
            val key = sbn.key ?: return null
            if (!isValidKey(key)) return null
            val app = truncateTo(appName(context, sbn), MAX_APP)
            val extras = sbn.notification.extras
            val title = truncateTo(
                extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty(), MAX_TITLE,
            )
            val body = truncateTo(
                extras.getCharSequence(Notification.EXTRA_BIG_TEXT)?.toString()
                    ?: extras.getCharSequence(Notification.EXTRA_TEXT)?.toString().orEmpty(),
                MAX_BODY,
            )
            val platformActions = sbn.notification.actions?.toList() ?: emptyList()
            val actions = platformActions.take(MAX_ACTIONS).mapIndexedNotNull { index, action ->
                val label = action.title?.toString()?.take(MAX_ACTION_LABEL)
                if (label.isNullOrEmpty()) null
                else WireNotificationAction(index.toString(), label)
            }
            val replySupported = platformActions.any { action ->
                action.remoteInputs?.any { it.allowFreeFormInput } == true
            }
            return WireNotification(
                key = key,
                app = app,
                title = title,
                body = body,
                clearable = sbn.isClearable,
                actions = actions,
                replySupported = replySupported,
            )
        }

        private fun appName(context: Context, sbn: StatusBarNotification): String {
            return runCatching {
                val pm = context.packageManager
                val info = pm.getApplicationInfo(sbn.packageName, 0)
                pm.getApplicationLabel(info).toString().ifEmpty { sbn.packageName }
            }.getOrDefault(sbn.packageName)
        }

        fun dismiss(key: String): Boolean {
            if (!isValidKey(key)) return false
            val svc = instance ?: return false
            return runCatching {
                svc.cancelNotification(key)
                true
            }.getOrDefault(false)
        }

        fun invokeAction(key: String, actionId: String): Boolean {
            if (!isValidKey(key) || actionId.isEmpty() || actionId.length > MAX_ACTION_ID) {
                return false
            }
            val svc = instance ?: return false
            val index = actionId.toIntOrNull() ?: return false
            val sbn = runCatching { svc.activeNotifications }.getOrNull()
                ?.firstOrNull { it.key == key } ?: return false
            val actions = sbn.notification.actions ?: return false
            if (index !in actions.indices) return false
            // Only advertise actions the platform reported; the index id maps
            // back to the same action without inventing new capabilities.
            return runCatching {
                actions[index].actionIntent.send()
                true
            }.getOrDefault(false)
        }

        fun reply(key: String, text: String): Boolean {
            if (!isValidKey(key) || text.isBlank() || text.length > MAX_REPLY_TEXT) return false
            val svc = instance ?: return false
            val sbn = runCatching { svc.activeNotifications }.getOrNull()
                ?.firstOrNull { it.key == key } ?: return false
            val action = sbn.notification.actions?.firstOrNull { candidate ->
                candidate.remoteInputs?.any { it.allowFreeFormInput } == true
            } ?: return false
            val inputs = action.remoteInputs ?: return false
            return runCatching {
                val intent = Intent()
                val bundle = Bundle()
                for (remoteInput in inputs) {
                    if (remoteInput.allowFreeFormInput) {
                        bundle.putCharSequence(remoteInput.resultKey, text)
                    }
                }
                RemoteInput.addResultsToIntent(inputs, intent, bundle)
                action.actionIntent.send(svc, 0, intent)
                true
            }.getOrDefault(false)
        }

        fun postJson(notification: WireNotification): JSONObject =
            JSONObject(postMap(notification))

        /** Pure map form of [postJson] for unit tests (android.jar stubs out
         * org.json under local unit tests, so assertions target the map). */
        fun postMap(notification: WireNotification): Map<String, Any?> = mapOf(
            "type" to "notification_post",
            "protocol" to 1,
            "key" to notification.key,
            "app" to notification.app,
            "title" to notification.title,
            "body" to notification.body,
            "clearable" to notification.clearable,
            "actions" to notification.actions.map { mapOf("id" to it.id, "label" to it.label) },
            "reply_supported" to notification.replySupported,
        )

        fun syncJson(enabled: Boolean, notifications: List<WireNotification>): JSONObject =
            JSONObject(syncMap(enabled, notifications))

        /** Pure map form of [syncJson] for unit tests. */
        fun syncMap(enabled: Boolean, notifications: List<WireNotification>): Map<String, Any?> =
            mapOf(
                "type" to "notifications_sync",
                "protocol" to 1,
                "enabled" to enabled,
                "notifications" to notifications.map {
                    mapOf(
                        "key" to it.key,
                        "app" to it.app,
                        "title" to it.title,
                        "body" to it.body,
                        "clearable" to it.clearable,
                        "actions" to it.actions.map { action ->
                            mapOf("id" to action.id, "label" to action.label)
                        },
                        "reply_supported" to it.replySupported,
                    )
                },
            )
    }
}
