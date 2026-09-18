package org.handover.android

import android.content.ContentResolver
import android.content.ContentValues
import android.content.Context
import android.annotation.SuppressLint
import android.net.Uri
import android.os.Build
import android.provider.ContactsContract
import android.provider.MediaStore
import android.util.Base64
import org.json.JSONArray
import org.json.JSONObject

/** Explicit developer diagnostic. The caller chooses where to share the dump. */
object ContactsDump {
    @SuppressLint("NewApi")
    fun export(context: Context): Uri? = runCatching {
        val root = JSONObject()
        root.put("contacts", query(context.contentResolver, ContactsContract.Contacts.CONTENT_URI))
        root.put("data", query(context.contentResolver, ContactsContract.Data.CONTENT_URI))
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, "handover-contact-provider-dump.json")
            put(MediaStore.Downloads.MIME_TYPE, "application/json")
            if (Build.VERSION.SDK_INT >= 29) put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val uri = context.contentResolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
            ?: return@runCatching null
        context.contentResolver.openOutputStream(uri)?.use { output ->
            output.write(root.toString(2).toByteArray(Charsets.UTF_8))
        } ?: return@runCatching null
        if (Build.VERSION.SDK_INT >= 29) {
            context.contentResolver.update(uri, ContentValues().apply {
                put(MediaStore.Downloads.IS_PENDING, 0)
            }, null, null)
        }
        uri
    }.getOrNull()

    private fun query(resolver: ContentResolver, uri: Uri): JSONArray {
        val rows = JSONArray()
        resolver.query(uri, null, null, null, null)?.use { cursor ->
            val columns = cursor.columnNames
            while (cursor.moveToNext()) {
                val row = JSONObject()
                for (index in columns.indices) {
                    if (cursor.isNull(index)) {
                        row.put(columns[index], JSONObject.NULL)
                    } else when (cursor.getType(index)) {
                        android.database.Cursor.FIELD_TYPE_BLOB -> row.put(
                            columns[index], Base64.encodeToString(cursor.getBlob(index), Base64.NO_WRAP),
                        )
                        android.database.Cursor.FIELD_TYPE_FLOAT -> row.put(columns[index], cursor.getDouble(index))
                        android.database.Cursor.FIELD_TYPE_INTEGER -> row.put(columns[index], cursor.getLong(index))
                        else -> row.put(columns[index], cursor.getString(index))
                    }
                }
                rows.put(row)
            }
        }
        return rows
    }
}
