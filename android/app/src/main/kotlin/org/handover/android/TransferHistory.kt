package org.handover.android

import android.content.Context
import android.content.Intent
import android.net.Uri
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

data class TransferRecord(
    val id: String,
    val kind: String,
    val name: String,
    val uri: Uri?,
    val status: String,
    val timestamp: Long,
)

object TransferHistory {
    private const val PREFS = "handover_transfers"
    private const val RECORDS = "records"
    private const val LIMIT = 20

    fun add(context: Context, kind: String, name: String, uri: Uri?, status: String = "received") {
        save(context, listOf(TransferRecord(
            UUID.randomUUID().toString(), kind, name, uri, status, System.currentTimeMillis(),
        )) + read(context))
    }

    /** Adds or updates an outgoing transfer using only metadata supplied by the transport. */
    fun recordResult(
        context: Context,
        transferId: String,
        status: String,
        kind: String?,
        name: String?,
        uri: Uri?,
    ) {
        if (transferId.isBlank() || status !in setOf("accepted", "completed", "failed")) return
        val records = read(context).toMutableList()
        val index = records.indexOfFirst { it.id == transferId }
        val previous = records.getOrNull(index)
        val record = TransferRecord(
            transferId,
            kind?.takeIf(String::isNotBlank) ?: previous?.kind ?: "transfer",
            name?.takeIf(String::isNotBlank) ?: previous?.name ?: transferId,
            uri ?: previous?.uri,
            status,
            System.currentTimeMillis(),
        )
        if (index >= 0) records.removeAt(index)
        save(context, listOf(record) + records)
    }

    fun read(context: Context): List<TransferRecord> = runCatching {
        val json = JSONArray(context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getString(RECORDS, "[]"))
        (0 until json.length()).mapNotNull { index ->
            val item = json.optJSONObject(index) ?: return@mapNotNull null
            TransferRecord(
                item.optString("id").ifEmpty { "legacy-$index-${item.optString("name").hashCode()}" },
                item.optString("kind", "file"),
                item.optString("name", "unnamed"),
                item.optString("uri").takeIf { it.isNotEmpty() && it != "null" }?.let(Uri::parse),
                item.optString("status", "received"),
                item.optLong("timestamp", 0L),
            )
        }
    }.getOrDefault(emptyList())

    fun remove(context: Context, id: String) = save(context, read(context).filterNot { it.id == id })

    fun clear(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        .edit().remove(RECORDS).apply()

    fun open(context: Context, record: TransferRecord): Boolean {
        val uri = record.uri ?: return false
        if (record.kind == "url" && !isOpenableUrl(uri)) return false
        val flags = Intent.FLAG_ACTIVITY_NEW_TASK or
            if (record.kind == "url") 0 else Intent.FLAG_GRANT_READ_URI_PERMISSION
        return runCatching {
            val intent = if (record.kind == "url") {
                Intent(Intent.ACTION_VIEW).setData(uri)
            } else {
                Intent(Intent.ACTION_VIEW).setDataAndType(uri,
                    context.contentResolver.getType(uri) ?: "application/octet-stream")
            }
            context.startActivity(intent.addFlags(flags))
            true
        }.getOrDefault(false)
    }

    internal fun isOpenableUrl(uri: Uri): Boolean =
        isOpenableUrlValue(uri.scheme)

    internal fun isOpenableUrlValue(scheme: String?): Boolean =
        scheme?.lowercase() == "http" || scheme?.lowercase() == "https"

    private fun save(context: Context, records: List<TransferRecord>) {
        val json = JSONArray()
        records.take(LIMIT).forEach { record ->
            json.put(JSONObject().put("id", record.id).put("kind", record.kind)
                .put("name", record.name).put("status", record.status)
                .put("timestamp", record.timestamp).apply {
                    record.uri?.let { put("uri", it.toString()) }
                })
        }
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putString(RECORDS, json.toString()).apply()
    }
}
