package org.handover.android

import android.os.BatteryManager
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class BatteryObserverTest {
    @Test fun parsesBatteryAndChargingState() {
        assertEquals(BatteryReading(42, true), BatteryObserver.reading(42, 100, BatteryManager.BATTERY_STATUS_CHARGING))
    }

    @Test fun rejectsMissingScale() {
        assertNull(BatteryObserver.reading(42, 0, BatteryManager.BATTERY_STATUS_UNKNOWN))
    }
}
