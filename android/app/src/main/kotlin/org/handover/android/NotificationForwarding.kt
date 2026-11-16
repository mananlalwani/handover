package org.handover.android

import android.content.Context
import android.content.Intent
import android.graphics.drawable.Drawable

/** Phone-local forwarding choices. Android notification delivery stays unchanged. */
internal class NotificationForwarding(context: Context) {
    private val context = context.applicationContext
    private val preferences = this.context.getSharedPreferences("notification_forwarding", Context.MODE_PRIVATE)

    fun isAllowed(packageName: String): Boolean = packageName !in blockedPackages()

    fun setAllowed(packageName: String, allowed: Boolean) {
        setAllowed(listOf(packageName), allowed)
    }

    /** Save a bulk choice once, then reconcile forwarded notifications once. */
    fun setAllowed(packageNames: Collection<String>, allowed: Boolean) {
        val blocked = blockedPackages().toMutableSet()
        if (allowed) blocked.removeAll(packageNames.toSet()) else blocked.addAll(packageNames)
        preferences.edit().putStringSet("blocked_packages", blocked).apply()
        HandoverNotificationService.requestSync(context)
    }

    fun remember(packageName: String) {
        val known = preferences.getStringSet("known_packages", emptySet()).orEmpty()
        if (packageName in known || known.size >= 512) return
        preferences.edit().putStringSet("known_packages", known + packageName).apply()
    }

    private fun blockedPackages(): Set<String> =
        preferences.getStringSet("blocked_packages", emptySet()).orEmpty()

    data class App(val packageName: String, val label: String, val icon: Drawable)

    /** Launcher apps plus notification senders, including apps without a launcher. */
    fun apps(): List<App> {
        val pm = context.packageManager
        val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
        val packages = pm.queryIntentActivities(launcher, 0).map { it.activityInfo.packageName }.toMutableSet()
        packages += preferences.getStringSet("known_packages", emptySet()).orEmpty()
        packages += blockedPackages()
        packages += HandoverNotificationService.activePackages()
        return packages.map { packageName ->
            val label = runCatching {
                pm.getApplicationLabel(pm.getApplicationInfo(packageName, 0)).toString()
            }.getOrDefault(packageName)
            val icon = runCatching { pm.getApplicationIcon(packageName) }.getOrElse { pm.defaultActivityIcon }
            App(packageName, label, icon)
        }.sortedWith(compareBy<App> { it.label.lowercase() }.thenBy { it.packageName })
    }
}
