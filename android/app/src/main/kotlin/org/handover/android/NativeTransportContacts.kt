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
import org.handover.android.NativeTransport.Companion.MAX_CHUNKED_CONTACTS
import org.handover.android.NativeTransport.Companion.MAX_CHUNKED_CONTACT_BYTES
import org.handover.android.NativeTransport.Companion.MAX_CHUNKED_CONTACT_CHUNKS

fun NativeTransport.requestContactsSync(chunked: Boolean = false): Boolean {
    if (serverFingerprint == null) return false
    val generation = if (chunked) java.util.UUID.randomUUID().toString() else null
    val requestSocket = if (chunked) socket else null
    val requestOutput = if (chunked) output else null
    if (chunked && (requestSocket == null || requestOutput == null)) return false
    fun fail(failure: String) {
        if (generation != null && requestSocket != null && requestOutput != null) {
            queueContactsError(contactErrorFrame(generation, failure), requestSocket, requestOutput)
        }
    }
    if (context.checkSelfPermission("android.permission.READ_CONTACTS") !=
        android.content.pm.PackageManager.PERMISSION_GRANTED
    ) {
        fail("permission_denied")
        return false
    }

    val snapshot = try {
        readContacts(chunked)
    } catch (_: ContactSnapshotTooLarge) {
        fail("too_large")
        return false
    } catch (_: SecurityException) {
        fail("permission_denied")
        return false
    } catch (_: Exception) {
        fail("rejected")
        return false
    } ?: run {
        fail("rejected")
        return false
    }

    if (!chunked) {
        send(JSONObject().put("type", "contacts_sync").put("protocol", 1)
            .put("complete", !snapshot.truncated)
            .put("contacts", org.json.JSONArray().apply { snapshot.contacts.forEach(::put) }))
        return true
    }

    val frames = buildContactChunks(generation!!, snapshot.contacts)
        ?: run {
            fail("too_large")
            return false
        }
    if (!queueContactsBatch(frames, requestSocket!!, requestOutput!!)) {
        fail("rejected")
        return false
    }
    return true
}

private data class ContactSnapshot(val contacts: List<JSONObject>, val truncated: Boolean)

private fun NativeTransport.readContacts(chunked: Boolean): ContactSnapshot? {
    val contacts = mutableListOf<JSONObject>()
    var usedBytes = 0
    var truncated = false
    val projection = arrayOf(
        ContactsContract.Contacts._ID,
        ContactsContract.Contacts.DISPLAY_NAME,
        ContactsContract.Contacts.PHOTO_THUMBNAIL_URI,
        ContactsContract.Contacts.PHOTO_URI,
        ContactsContract.Contacts.PHOTO_FILE_ID,
        ContactsContract.Contacts.LOOKUP_KEY,
    )
    val cursor = context.contentResolver.query(
        ContactsContract.Contacts.CONTENT_URI, projection, null, null,
        ContactsContract.Contacts.DISPLAY_NAME + " COLLATE NOCASE ASC",
    ) ?: return if (chunked) null else ContactSnapshot(emptyList(), false)
    cursor.use {
        val idIndex = it.getColumnIndexOrThrow(ContactsContract.Contacts._ID)
        val nameIndex = it.getColumnIndexOrThrow(ContactsContract.Contacts.DISPLAY_NAME)
        val photoIndex = it.getColumnIndexOrThrow(ContactsContract.Contacts.PHOTO_THUMBNAIL_URI)
        val fullPhotoIndex = it.getColumnIndexOrThrow(ContactsContract.Contacts.PHOTO_URI)
        val photoFileIndex = it.getColumnIndexOrThrow(ContactsContract.Contacts.PHOTO_FILE_ID)
        val lookupIndex = it.getColumnIndexOrThrow(ContactsContract.Contacts.LOOKUP_KEY)
        while (it.moveToNext()) {
            if (chunked && contacts.size >= MAX_CHUNKED_CONTACTS) throw ContactSnapshotTooLarge()
            val id = it.getString(idIndex)
            val item = JSONObject().put("local_id", id.take(MAX_CONTACT_FIELD_CHARS))
                .put("display_name", (it.getString(nameIndex) ?: "").take(MAX_CONTACT_FIELD_CHARS))
            val phones = org.json.JSONArray()
            val phoneCursorResult = context.contentResolver.query(
                ContactsContract.CommonDataKinds.Phone.CONTENT_URI,
                arrayOf(ContactsContract.CommonDataKinds.Phone.NUMBER),
                "${ContactsContract.CommonDataKinds.Phone.CONTACT_ID}=?", arrayOf(id), null,
            )
            if (chunked && phoneCursorResult == null) throw IllegalStateException("phone query failed")
            phoneCursorResult?.use { phoneCursor ->
                while (phoneCursor.moveToNext() && phones.length() < MAX_CONTACT_VALUES) {
                    phoneCursor.getString(0)?.let { value ->
                        phones.put(value.take(MAX_CONTACT_FIELD_CHARS))
                    }
                }
            }
            val emails = org.json.JSONArray()
            val emailCursorResult = context.contentResolver.query(
                ContactsContract.CommonDataKinds.Email.CONTENT_URI,
                arrayOf(ContactsContract.CommonDataKinds.Email.ADDRESS),
                "${ContactsContract.CommonDataKinds.Email.CONTACT_ID}=?", arrayOf(id), null,
            )
            if (chunked && emailCursorResult == null) throw IllegalStateException("email query failed")
            emailCursorResult?.use { emailCursor ->
                while (emailCursor.moveToNext() && emails.length() < MAX_CONTACT_VALUES) {
                    emailCursor.getString(0)?.let { value ->
                        emails.put(value.take(MAX_CONTACT_FIELD_CHARS))
                    }
                }
            }
            item.put("phones", phones).put("emails", emails)
            val thumbnailPhotoUri = it.getString(photoIndex)
            val fullPhotoUri = it.getString(fullPhotoIndex)
            val photoFileId = it.getLong(photoFileIndex).takeIf { value -> value > 0 }
            if (thumbnailPhotoUri != null || fullPhotoUri != null || photoFileId != null) {
                val lookupKey = it.getString(lookupIndex)
                val photo = encodeContactPhoto(
                    id,
                    thumbnailPhotoUri?.let(Uri::parse) ?: fullPhotoUri?.let(Uri::parse),
                    photoFileId,
                    lookupKey,
                )
                val encodedPhotoBytes = photo?.toByteArray(Charsets.UTF_8)?.size ?: 0
                val withinPhotoLimit = if (chunked) {
                    encodedPhotoBytes <= 48 * 1024
                } else {
                    usedBytes + encodedPhotoBytes <= 48 * 1024
                }
                if (!photo.isNullOrEmpty() && withinPhotoLimit) {
                    item.put("photo", photo)
                }
            }
            val itemBytes = item.toString().toByteArray(Charsets.UTF_8).size
            if (chunked) {
                // Match the receiver's conservative record accounting exactly.
                if (usedBytes + itemBytes + 1 > MAX_CHUNKED_CONTACT_BYTES) {
                    throw ContactSnapshotTooLarge()
                }
            } else if (usedBytes + itemBytes + 1 > CONTACTS_BUDGET_BYTES) {
                android.util.Log.i("Handover", "contacts snapshot truncated to frame budget")
                truncated = true
                break
            }
            contacts.add(item)
            usedBytes += itemBytes + 1
        }
    }
    return ContactSnapshot(contacts, truncated)
}

private class ContactSnapshotTooLarge : RuntimeException()

private fun buildContactChunks(generation: String, contacts: List<JSONObject>): List<JSONObject>? {
    val chunks = mutableListOf<JSONObject>()
    var current = mutableListOf<JSONObject>()
    for (contact in contacts) {
        val candidate = current + contact
        val candidateFrame = contactChunkFrame(generation, chunks.size, false, candidate)
        if (candidateFrame.toString().toByteArray(Charsets.UTF_8).size > CONTACTS_BUDGET_BYTES) {
            if (current.isEmpty()) return null
            chunks.add(contactChunkFrame(generation, chunks.size, false, current))
            if (chunks.size >= MAX_CHUNKED_CONTACT_CHUNKS) return null
            current = mutableListOf(contact)
            if (contactChunkFrame(generation, chunks.size, false, current)
                    .toString().toByteArray(Charsets.UTF_8).size > CONTACTS_BUDGET_BYTES
            ) return null
        } else {
            current.add(contact)
        }
    }
    chunks.add(contactChunkFrame(generation, chunks.size, true, current))
    if (chunks.size > MAX_CHUNKED_CONTACT_CHUNKS) return null
    return chunks
}

private fun contactChunkFrame(
    generation: String,
    index: Int,
    done: Boolean,
    contacts: List<JSONObject>,
) = JSONObject().put("type", "contacts_chunk").put("protocol", 1)
    .put("generation", generation).put("index", index).put("done", done)
    .put("contacts", org.json.JSONArray().apply { contacts.forEach(::put) })

private fun contactErrorFrame(generation: String, failure: String) =
    JSONObject().put("type", "contacts_error").put("protocol", 1)
        .put("generation", generation).put("failure", failure)

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
