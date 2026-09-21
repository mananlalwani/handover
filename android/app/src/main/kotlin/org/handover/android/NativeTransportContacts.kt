package org.handover.android

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.provider.ContactsContract
import android.util.Base64
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import org.handover.android.NativeTransport.Companion.CONTACTS_BUDGET_BYTES
import org.handover.android.NativeTransport.Companion.MAX_CONTACT_FIELD_CHARS
import org.handover.android.NativeTransport.Companion.MAX_CONTACT_VALUES

fun NativeTransport.requestContactsSync(): Boolean {
    if (serverFingerprint == null || context.checkSelfPermission("android.permission.READ_CONTACTS") !=
        android.content.pm.PackageManager.PERMISSION_GRANTED) return false
    // Explicit bounded snapshot contract: the whole list must fit
    // one 64 KiB frame. Names, numbers, and emails stop accumulating
    // once the serialized list nears the budget (photos already
    // stop at 48 KiB), so a large address book truncates instead of
    // throwing inside the writer and killing the sync.
    val contacts = org.json.JSONArray()
    var truncated = false
    var usedBytes = 0
    val projection = arrayOf(
        ContactsContract.Contacts._ID,
        ContactsContract.Contacts.DISPLAY_NAME,
        ContactsContract.Contacts.PHOTO_THUMBNAIL_URI,
        ContactsContract.Contacts.PHOTO_URI,
        ContactsContract.Contacts.PHOTO_FILE_ID,
        ContactsContract.Contacts.LOOKUP_KEY,
    )
    context.contentResolver.query(
        ContactsContract.Contacts.CONTENT_URI, projection, null, null,
        ContactsContract.Contacts.DISPLAY_NAME + " COLLATE NOCASE ASC",
    )?.use { cursor ->
        val idIndex = cursor.getColumnIndexOrThrow(ContactsContract.Contacts._ID)
        val nameIndex = cursor.getColumnIndexOrThrow(ContactsContract.Contacts.DISPLAY_NAME)
        val photoIndex = cursor.getColumnIndexOrThrow(ContactsContract.Contacts.PHOTO_THUMBNAIL_URI)
        val fullPhotoIndex = cursor.getColumnIndexOrThrow(ContactsContract.Contacts.PHOTO_URI)
        val photoFileIndex = cursor.getColumnIndexOrThrow(ContactsContract.Contacts.PHOTO_FILE_ID)
        val lookupIndex = cursor.getColumnIndexOrThrow(ContactsContract.Contacts.LOOKUP_KEY)
        while (cursor.moveToNext()) {
            // Budget in UTF-8 bytes: the wire frame is byte-limited
            // while String.length counts UTF-16 units, so CJK/emoji
            // heavy data could pass a char check and still burst
            // the frame and close the connection.
            val id = cursor.getString(idIndex)
            val item = JSONObject().put("local_id", id)
                .put("display_name", (cursor.getString(nameIndex) ?: "").take(MAX_CONTACT_FIELD_CHARS))
            val phones = org.json.JSONArray()
            context.contentResolver.query(
                ContactsContract.CommonDataKinds.Phone.CONTENT_URI,
                arrayOf(ContactsContract.CommonDataKinds.Phone.NUMBER),
                "${ContactsContract.CommonDataKinds.Phone.CONTACT_ID}=?", arrayOf(id), null,
            )?.use { phoneCursor ->
                while (phoneCursor.moveToNext() && phones.length() < MAX_CONTACT_VALUES) {
                    phoneCursor.getString(0)?.let { phones.put(it.take(MAX_CONTACT_FIELD_CHARS)) }
                }
            }
            item.put("phones", phones).put("emails", org.json.JSONArray())
            val emails = org.json.JSONArray()
            context.contentResolver.query(
                ContactsContract.CommonDataKinds.Email.CONTENT_URI,
                arrayOf(ContactsContract.CommonDataKinds.Email.ADDRESS),
                "${ContactsContract.CommonDataKinds.Email.CONTACT_ID}=?", arrayOf(id), null,
            )?.use { emailCursor ->
                while (emailCursor.moveToNext() && emails.length() < MAX_CONTACT_VALUES) {
                    emailCursor.getString(0)?.let { emails.put(it.take(MAX_CONTACT_FIELD_CHARS)) }
                }
            }
            item.put("emails", emails)
            val photoUri = cursor.getString(photoIndex)
            if (photoUri != null) {
                val photoFileId = cursor.getLong(photoFileIndex).takeIf { it > 0 }
                val lookupKey = cursor.getString(lookupIndex)
                val fullPhotoUri = cursor.getString(fullPhotoIndex)
                val photo = encodeContactPhoto(
                    id,
                    photoUri?.let(Uri::parse) ?: fullPhotoUri?.let(Uri::parse),
                    photoFileId,
                    lookupKey,
                )
                if (!photo.isNullOrEmpty() && usedBytes + photo.toByteArray(Charsets.UTF_8).size < 48 * 1024)
                    item.put("photo", photo)
            }
            val itemBytes = item.toString().toByteArray(Charsets.UTF_8).size + 1
            if (usedBytes + itemBytes > CONTACTS_BUDGET_BYTES) {
                truncated = true
                break
            }
            contacts.put(item)
            usedBytes += itemBytes
        }
    }
    if (truncated) android.util.Log.i("Handover", "contacts snapshot truncated to frame budget")
    send(JSONObject().put("type", "contacts_sync").put("protocol", 1).put("contacts", contacts))
    return true
}

internal fun NativeTransport.encodeContactPhoto(
    contactId: String,
    thumbnailUri: Uri?,
    photoFileId: Long?,
    lookupKey: String?,
): String? = runCatching {
    val contactUri = ContactsContract.Contacts.CONTENT_URI.buildUpon()
        .appendPath(contactId).build()
    val lookupUri = lookupKey?.let {
        ContactsContract.Contacts.getLookupUri(contactId.toLong(), it)
    }
    val fileUri = photoFileId?.let {
        ContactsContract.DisplayPhoto.CONTENT_URI.buildUpon().appendPath(it.toString()).build()
    }
    val input = thumbnailUri?.let { context.contentResolver.openInputStream(it) }
        ?: fileUri?.let { context.contentResolver.openInputStream(it) }
        ?: lookupUri?.let {
            ContactsContract.Contacts.openContactPhotoInputStream(context.contentResolver, it, true)
        }
        ?: ContactsContract.Contacts.openContactPhotoInputStream(
            context.contentResolver, contactUri, true,
        )
    val bitmap = input?.use(BitmapFactory::decodeStream) ?: context.contentResolver.query(
        ContactsContract.Data.CONTENT_URI,
        arrayOf(
            ContactsContract.Data.DATA15,
            ContactsContract.CommonDataKinds.Photo.PHOTO_FILE_ID,
        ),
        "${ContactsContract.Data.CONTACT_ID}=? AND ${ContactsContract.Data.MIMETYPE}=?",
        arrayOf(
            contactId,
            ContactsContract.CommonDataKinds.Photo.CONTENT_ITEM_TYPE,
        ),
        null,
    )?.use { cursor ->
        if (!cursor.moveToFirst()) return@use null
        if (!cursor.isNull(0)) {
            val blob = cursor.getBlob(0)
            BitmapFactory.decodeByteArray(blob, 0, blob.size)
        } else if (!cursor.isNull(1)) {
            val id = cursor.getLong(1)
            val uri = ContactsContract.DisplayPhoto.CONTENT_URI.buildUpon()
                .appendPath(id.toString()).build()
            context.contentResolver.openInputStream(uri)?.use(BitmapFactory::decodeStream)
        } else null
    }
        ?: return@runCatching null
    val size = maxOf(bitmap.width, bitmap.height)
    val scaled = if (size > 96) {
        val scale = 96f / size
        Bitmap.createScaledBitmap(
            bitmap, (bitmap.width * scale).toInt().coerceAtLeast(1),
            (bitmap.height * scale).toInt().coerceAtLeast(1), true,
        )
    } else bitmap
    ByteArrayOutputStream().use { output ->
        if (!scaled.compress(Bitmap.CompressFormat.JPEG, 70, output)) return@runCatching null
        Base64.encodeToString(output.toByteArray(), Base64.NO_WRAP)
    }.also {
        if (scaled !== bitmap) scaled.recycle()
        bitmap.recycle()
    }
}.getOrNull()
