package org.handover.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream

class SharedFolderPathTest {
    @Test fun acceptsRelativePathAndDotRoot() {
        assertEquals(listOf("Photos", "trip.jpg"), safeTreePathParts("Photos/trip.jpg"))
        assertEquals(emptyList<String>(), safeTreePathParts("."))
    }

    @Test fun rejectsTraversalAbsoluteAndAmbiguousSeparators() {
        listOf("", "a//b", "a/./b", "a/", "é".repeat(128), "../secret", "a/../../secret", "/etc/passwd", "a\\secret", "a\u0000b", "a//b", "a/./b", "").forEach {
            assertThrows(IllegalArgumentException::class.java) { safeTreePathParts(it) }
        }
    }

    @Test fun enforcesUtf8ByteLimit() {
        assertThrows(IllegalArgumentException::class.java) { safeTreePathParts("é".repeat(257)) }
    }

    @Test fun revocationDuringProviderReadDoesNotWriteItsBytes() {
        var active = true
        val input = object : ByteArrayInputStream(byteArrayOf(1, 2, 3)) {
            override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
                val count = super.read(buffer, offset, length)
                active = false
                return count
            }
        }
        val output = ByteArrayOutputStream()
        assertThrows(SecurityException::class.java) {
            copyBounded(input, output, 10) { if (!active) throw SecurityException() }
        }
        assertEquals(0, output.size())
    }

    @Test fun copiesOriginalBytesAndEnforcesLimit() {
        val original = byteArrayOf(0, 1, 2, -1, 10)
        val output = ByteArrayOutputStream()
        assertEquals(original.size.toLong(), copyBounded(ByteArrayInputStream(original), output, original.size.toLong()))
        assertEquals(original.toList(), output.toByteArray().toList())
        assertThrows(java.io.IOException::class.java) {
            copyBounded(ByteArrayInputStream(original), ByteArrayOutputStream(), 4)
        }
    }
}
