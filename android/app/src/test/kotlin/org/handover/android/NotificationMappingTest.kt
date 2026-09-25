package org.handover.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NotificationMappingTest {
    @Test fun keyValidationMatchesNativeBounds() {
        assertFalse(HandoverNotificationService.isValidKey(""))
        assertFalse(HandoverNotificationService.isValidKey("a".repeat(257)))
        assertFalse(HandoverNotificationService.isValidKey("bad\u0000key"))
        assertTrue(HandoverNotificationService.isValidKey("0|com.example|42|tag|10042"))
    }

    @Test fun truncationMirrorsNativeBounds() {
        assertEquals("abc", HandoverNotificationService.truncateTo("abcdef", 3))
        assertEquals("abc", HandoverNotificationService.truncateTo("abc", 3))
        assertEquals(
            HandoverNotificationService.MAX_TITLE,
            HandoverNotificationService.truncateTo(
                "t".repeat(HandoverNotificationService.MAX_TITLE + 10),
                HandoverNotificationService.MAX_TITLE,
            ).length,
        )
    }

    @Test fun postMapUsesVersionedType() {
        val notification = WireNotification(
            key = "key-1",
            app = "Example",
            title = "Hello",
            body = "World",
            clearable = true,
            actions = listOf(WireNotificationAction("0", "Reply")),
            replySupported = true,
        )
        val map = HandoverNotificationService.postMap(notification)
        assertEquals("notification_post", map["type"])
        assertEquals(1, map["protocol"])
        assertEquals("key-1", map["key"])
        @Suppress("UNCHECKED_CAST")
        val actions = map["actions"] as List<Map<String, String>>
        assertEquals("Reply", actions[0]["label"])
        assertEquals(true, map["reply_supported"])
    }

    @Test fun syncMapReportsPermissionState() {
        val disabled = HandoverNotificationService.syncMap(false, emptyList())
        assertEquals("notifications_sync", disabled["type"])
        assertEquals(false, disabled["enabled"])
        @Suppress("UNCHECKED_CAST")
        assertTrue((disabled["notifications"] as List<*>).isEmpty())

        val enabled = HandoverNotificationService.syncMap(
            true,
            listOf(
                WireNotification("k", "a", "t", "b", true, emptyList(), false),
            ),
        )
        assertEquals(true, enabled["enabled"])
        @Suppress("UNCHECKED_CAST")
        val list = enabled["notifications"] as List<Map<String, Any?>>
        assertEquals("t", list[0]["title"])
    }

    @Test fun commandValidationRejectsBadKeysAndOversizeReply() {
        assertFalse(HandoverNotificationService.dismiss(""))
        assertFalse(HandoverNotificationService.invokeAction("key-1", ""))
        // No listener instance is bound in unit tests, so a well-formed
        // request reports false (no service) rather than throwing.
        assertFalse(HandoverNotificationService.dismiss("key-1"))
        assertFalse(HandoverNotificationService.reply("key-1", "x".repeat(1025)))
        assertFalse(HandoverNotificationService.reply("key-1", "   "))
    }
}
