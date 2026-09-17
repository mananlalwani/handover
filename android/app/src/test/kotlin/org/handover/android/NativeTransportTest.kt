package org.handover.android

import org.junit.Assert.assertEquals
import org.junit.Test

class NativeTransportTest {
    @Test fun pairingCodeMatchesCrossLanguageVectorAndIsOrderIndependent() {
        // Shared with handover-native's comparison_code unit test: the same
        // fingerprints and nonces must produce this code in both languages.
        val fpA = "aa".repeat(32)
        val fpB = "bb".repeat(32)
        val nonceA = "00112233445566778899aabbccddeeff"
        val nonceB = "ffeeddccbbaa99887766554433221100"
        val code = NativeTransport.pairingCode(fpA, nonceA, fpB, nonceB)
        assertEquals("18954386", code)
        assertEquals(code, NativeTransport.pairingCode(fpB, nonceB, fpA, nonceA))
    }

    @Test fun pairingCommitmentBindsNonceOpening() {
        val nonce = "00112233445566778899aabbccddeeff"
        assertEquals(
            "a8faed6abbf35c12a4b26e40f6feb19d736d90045c83b9f9a31f638d323e6811",
            NativeTransport.pairingCommitment(nonce)
        )
        assertEquals(8, NativeTransport.pairingCode(fpOf("11"), nonce, fpOf("22"), nonce).length)
    }

    private fun fpOf(prefix: String) = prefix.repeat(32)
}
