package org.handover.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class NativeTransportTest {
    @Test fun pairingCodeIsEightDigitsAndOrderIndependent() {
        val first = NativeTransport.pairingCode("a", "b")
        assertEquals(8, first.length)
        assertTrue(first.all { it in '0'..'9' })
        assertEquals(first, NativeTransport.pairingCode("b", "a"))
    }
}
