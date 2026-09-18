package org.handover.android

import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import org.junit.Assert.assertThrows
import org.junit.Test

class PinnedIdentityTrustManagerTest {
    @Test
    fun rejectsEmptyServerChainBeforePairing() {
        assertThrows(CertificateException::class.java) {
            PinnedIdentityTrustManager(null).checkServerTrusted(emptyArray(), "RSA")
        }
    }

    @Test
    fun rejectsEmptyClientChain() {
        assertThrows(CertificateException::class.java) {
            PinnedIdentityTrustManager(null).checkClientTrusted(emptyArray(), "RSA")
        }
    }
}
