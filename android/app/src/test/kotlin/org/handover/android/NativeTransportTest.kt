package org.handover.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class NativeTransportTest {
    @Test fun remoteInputBoundsRejectIntMinValueAndAcceptInclusiveEdges() {
        assertEquals(false, NativeTransport.isWithinRemoteInputBounds(Int.MIN_VALUE, 0))
        assertEquals(false, NativeTransport.isWithinRemoteInputBounds(0, Int.MIN_VALUE))
        assertEquals(true, NativeTransport.isWithinRemoteInputBounds(-2000, 2000))
        assertEquals(false, NativeTransport.isWithinRemoteInputBounds(-2001, 0))
    }

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

    @Test fun savedEndpointPrefersManualThenLast() {
        assertEquals(
            "10.0.0.2:24837",
            NativeTransport.savedEndpointAddress("10.0.0.2:24837", "100.64.0.1:24837"),
        )
        assertEquals("100.64.0.1:24837", NativeTransport.savedEndpointAddress(null, "100.64.0.1:24837"))
        assertEquals(null, NativeTransport.savedEndpointAddress("  ", null))
        assertEquals(true, NativeTransport.endpointIsManual("10.0.0.2:24837", "10.0.0.2:24837"))
        assertEquals(false, NativeTransport.endpointIsManual("10.0.0.2:24837", "100.64.0.1:24837"))
        assertEquals(false, NativeTransport.endpointIsManual(null, "100.64.0.1:24837"))
    }

    @Test fun autoStartsOnlyWhenAPeerIsPaired() {
        assertEquals(true, NativeTransport.shouldAutoStartService("aa".repeat(32)))
        assertEquals(false, NativeTransport.shouldAutoStartService(null))
        assertEquals(false, NativeTransport.shouldAutoStartService(""))
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

    @Test fun browsePathsRejectTraversal() {
        assertEquals(true, NativeTransport.isSafeBrowsePath("."))
        assertEquals(true, NativeTransport.isSafeBrowsePath("Documents/notes"))
        assertEquals(false, NativeTransport.isSafeBrowsePath("/etc"))
        assertEquals(false, NativeTransport.isSafeBrowsePath("../secret"))
        assertEquals(false, NativeTransport.isSafeBrowsePath("foo/../bar"))
    }

    @Test fun commandNamesMatchLinuxAllowlistRules() {
        assertEquals(true, NativeTransport.isSafeCommandName("lock-screen"))
        assertEquals(false, NativeTransport.isSafeCommandName("Bad Name!"))
        assertEquals(false, NativeTransport.isSafeCommandName("has.dot"))
    }

    private fun fpOf(prefix: String) = prefix.repeat(32)
}
