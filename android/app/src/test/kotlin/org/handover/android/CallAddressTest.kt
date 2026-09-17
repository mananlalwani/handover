package org.handover.android

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CallAddressTest {
    @Test fun acceptsOnlyPlainDialAddresses() {
        listOf("1", "+123456789012345", "1234567").forEach { assertTrue(validCallAddress(it)) }
        listOf("", "+", "++1", "１２３", "tel:123", "123;4", "123,4", "*#06#", " 123", "1234567890123456")
            .forEach { assertFalse(validCallAddress(it)) }
    }
}
