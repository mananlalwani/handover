package org.handover.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class NativeTransportTest {
    @Test fun deviceCommandResultReportsPermissionDenial() {
        val result = NativeTransport.deviceCommandResultFields(
            "0123456789abcdef0123456789abcdef", "lock", false, "permission_denied",
        )

        assertEquals("device_command_result", result["type"])
        assertEquals("lock", result["action"])
        assertFalse(result["accepted"] as Boolean)
        assertEquals("permission_denied", result["failure"])
    }

    @Test fun notificationResultReportsAndroidApplication() {
        val result = NativeTransport.deviceCommandResultFields(
            "0123456789abcdef0123456789abcdef", "notification", true, null,
        )

        assertEquals("notification", result["action"])
        assertEquals(true, result["accepted"])
        assertFalse(result.containsKey("failure"))
    }

    @Test fun keepAwakeResultReportsWakeLockState() {
        val result = NativeTransport.deviceCommandResultFields(
            "0123456789abcdef0123456789abcdef", "keep_awake", true, null,
        )

        assertEquals("keep_awake", result["action"])
        assertEquals(true, result["accepted"])
        assertFalse(result.containsKey("failure"))
    }

    @Test fun tetheringResultReportsSettingsScreen() {
        val result = NativeTransport.deviceCommandResultFields(
            "0123456789abcdef0123456789abcdef", "tethering", true, null,
        )

        assertEquals("tethering", result["action"])
        assertEquals(true, result["accepted"])
        assertFalse(result.containsKey("failure"))
    }

    @Test fun overlayAssistRequiresDoubleOptInPlusGrantPlusDeniedRead() {
        assertEquals(
            true,
            NativeTransport.shouldAssistBackgroundRead(true, true, true, true),
        )
        assertEquals(
            false,
            NativeTransport.shouldAssistBackgroundRead(false, true, true, true),
        )
        assertEquals(
            false,
            NativeTransport.shouldAssistBackgroundRead(true, false, true, true),
        )
        assertEquals(
            false,
            NativeTransport.shouldAssistBackgroundRead(true, true, false, true),
        )
        assertEquals(
            false,
            NativeTransport.shouldAssistBackgroundRead(true, true, true, false),
        )
    }

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

    @Test fun receivedUrlsOnlyAllowHttpAndHttps() {
        assertEquals(true, TransferHistory.isOpenableUrlValue("https"))
        assertEquals(true, TransferHistory.isOpenableUrlValue("HTTP"))
        assertEquals(false, TransferHistory.isOpenableUrlValue("content"))
        assertEquals(false, TransferHistory.isOpenableUrlValue("javascript"))
    }

    @Test fun shareUrlValidationRejectsNonWebSchemes() {
        assertEquals(true, NativeTransport.isValidShareUrl("https://example.test"))
        assertEquals(true, NativeTransport.isValidShareUrl("http://example.test/path"))
        assertEquals(false, NativeTransport.isValidShareUrl("content://contacts/1"))
        assertEquals(false, NativeTransport.isValidShareUrl("file:///etc/passwd"))
    }

    @Test fun inboundTransferDeadlineIsHardBoundary() {
        assertEquals(false, NativeTransport.transferDeadlineExpired(100L, 99L))
        assertEquals(true, NativeTransport.transferDeadlineExpired(100L, 100L))
        assertEquals(30_000L, NativeTransport.remainingTransferTimeoutMillis(60_000_000_000L, 0L))
        assertEquals(1L, NativeTransport.remainingTransferTimeoutMillis(100L, 100L))
        assertEquals(2L, NativeTransport.remainingTransferTimeoutMillis(2_000_000L, 0L))
    }

    private fun fpOf(prefix: String) = prefix.repeat(32)
}
