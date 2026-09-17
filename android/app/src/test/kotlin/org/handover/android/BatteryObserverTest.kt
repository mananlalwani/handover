package org.handover.android

import android.os.BatteryManager
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.io.File

class BatteryObserverTest {
    @Test fun parsesBatteryAndChargingState() {
        assertEquals(BatteryReading(42, true), BatteryObserver.reading(42, 100, BatteryManager.BATTERY_STATUS_CHARGING))
    }

    @Test fun rejectsMissingScale() {
        assertNull(BatteryObserver.reading(42, 0, BatteryManager.BATTERY_STATUS_UNKNOWN))
    }

    @Test fun sharedNativeProtocolFixtureContainsVersionedMessages() {
        val fixture = File("../../tests/fixtures/native-protocol.json")
        check(fixture.isFile) { "shared protocol fixture is missing: ${fixture.absolutePath}" }
        val text = fixture.readText()
        val types = Regex("\\\"type\\\"\\s*:\\s*\\\"([^\\\"]+)\\\"")
            .findAll(text).map { it.groupValues[1] }.toList()
        assertEquals(listOf("hello", "pair_open", "pair_confirm", "paired", "battery", "notification_post", "notification_removed", "notifications_sync", "notifications_request", "notification_dismiss", "notification_reply", "notification_action", "media_post", "media_removed", "media_sync", "media_request", "media_control", "media_control", "share_url", "share_file", "share_result", "share_result", "revoke", "ping", "pong"), types)
        assertEquals(25, Regex("\\\"protocol\\\"\\s*:\\s*1").findAll(text).count())
    }
}
