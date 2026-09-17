package org.handover.android

import android.content.Context
import android.content.Intent
import android.net.Uri
import org.json.JSONArray
import org.json.JSONObject

data class TransferRecord(val kind: String, val name: String, val uri: Uri?, val status: String)

object TransferHistory {
    private const val PREFS = "handover_transfers"
    private const val RECORDS = "records"
    private const val LIMIT = 20

    fun add(context: Context, kind: String, name: String, uri: Uri?, status: String = "received") {
        val json = JSONArray()
        (listOf(TransferRecord(kind, name, uri, status)) + read(context)).take(LIMIT).forEach { record ->
            json.put(JSONObject().put("kind", record.kind).put("name", record.name)
                .put("uri", record.uri?.toString()).put("status", record.status))
        }
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putString(RECORDS, json.toString()).apply()
    }

    fun read(context: Context): List<TransferRecord> = runCatching {
        val json = JSONArray(context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getString(RECORDS, "[]"))
        (0 until json.length()).mapNotNull { index ->
            val item = json.optJSONObject(index) ?: return@mapNotNull null
            TransferRecord(item.optString("kind", "file"), item.optString("name", "unnamed"),
                item.optString("uri").takeIf(String::isNotEmpty)?.let(Uri::parse),
                item.optString("status", "received"))
        }
    }.getOrDefault(emptyList())

    fun clear(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        .edit().remove(RECORDS).apply()

    fun open(context: Context, record: TransferRecord): Boolean {
        val uri = record.uri ?: return false
        return runCatching {
            context.startActivity(Intent(Intent.ACTION_VIEW).setDataAndType(uri,
                if (record.kind == "url") "text/plain"
                else context.contentResolver.getType(uri) ?: "application/octet-stream")
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_GRANT_READ_URI_PERMISSION))
            true
        }.getOrDefault(false)
    }
}
