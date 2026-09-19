package org.handover.android

import java.security.cert.CertificateException
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import java.io.ByteArrayInputStream
import java.util.Base64
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

    @Test
    fun acceptsSelfSignedP256IdentityCertificate() {
        val certificate = certificate(EC_P256_CERTIFICATE)
        PinnedIdentityTrustManager(null).checkServerTrusted(arrayOf(certificate), "ECDHE_ECDSA")
    }

    @Test
    fun rejectsNonEcIdentityCertificate() {
        val certificate = certificate(RSA_CERTIFICATE)
        assertThrows(CertificateException::class.java) {
            PinnedIdentityTrustManager(null).checkServerTrusted(arrayOf(certificate), "RSA")
        }
    }

    private fun certificate(encoded: String): X509Certificate =
        CertificateFactory.getInstance("X.509").generateCertificate(
            ByteArrayInputStream(Base64.getDecoder().decode(encoded)),
        ) as X509Certificate

    companion object {
        private const val EC_P256_CERTIFICATE =
            "MIIBdjCCARugAwIBAgIUHyDm0DJov8D5Jp1O7p0edLQeLUQwCgYIKoZIzj0EAwIwDzENMAsGA1UEAwwEdGVzdDAgFw0yNjA5MTkyMzQ3MTRaGA8yMTI2MDgyNjIzNDcxNFowDzENMAsGA1UEAwwEdGVzdDBZMBMGByqGSM49AgEGCCqGSM49AwEHA0IABAa5VqIWy+qCsMmsFdKU3X3+JyiNhyaIMRA7w0oLL8Hj3wSKPkOECXJT/lKA5SCpVEQaFESuA8Ao6moMWCovpymjUzBRMB0GA1UdDgQWBBS9wBTzSkuFEX2rpd3U2mHFrxCBsDAfBgNVHSMEGDAWgBS9wBTzSkuFEX2rpd3U2mHFrxCBsDAPBgNVHRMBAf8EBTADAQH/MAoGCCqGSM49BAMCA0kAMEYCIQCrr63mWngEyJKBycH6OjNWhe+GeZ5z6Q9zKN2ev2rJnQIhAM9hzdr5FXiFkJVqF/uRK/YNwo4DEInfHUgwkRqMTN48"
        private const val RSA_CERTIFICATE =
            "MIIDATCCAemgAwIBAgIUYLzjQK6gRh1erV7EK+pcalXejmYwDQYJKoZIhvcNAQELBQAwDzENMAsGA1UEAwwEdGVzdDAgFw0yNjA5MTkyMzQ3MTRaGA8yMTI2MDgyNjIzNDcxNFowDzENMAsGA1UEAwwEdGVzdDCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBALDMj5StWzlQbqIa/GRs07jB0Zz+u2UbkN32x4Ej8l3U2+U05vzShYPZ5y35rAetTdnq2AnW6e9k/c57UA3lcyJ1MVeXm/m/ta7Eem0oEsx91HKdAlhILE1M3mdOaiJ/Q1FUzOuFDdvGmscS25/PewBzH54vAW3aBPpKUMaRzbA+MliMxr+ds2Z0Aa+nfYBb00eG0N3XxTQq5WhnfRK0ghHoZOHhICg592LuX3jzXzK2DcPH35SsQgQl1cPJJzgLcMWYLAgB6Ij4kCjwDtBdqbD+CcQAH1LsdL47rdERTFHRkpmwPUrthBcPBG+v/foT4VXv8sFK+uQ+Gy2IloJ+BpcCAwEAAaNTMFEwHQYDVR0OBBYEFF1RnWSqmDXxbSAD/G0Nix8rGA28MB8GA1UdIwQYMBaAFF1RnWSqmDXxbSAD/G0Nix8rGA28MA8GA1UdEwEB/wQFMAMBAf8wDQYJKoZIhvcNAQELBQADggEBAFqG6pqioZr0jF3Tfze2nGRAYYeAE1N6ctNbM0WklmbFzy9FDptWW/8QFuM012F4L+mhKUa0MpvOsdNj+0WjgVva1gn/SGZUgyNoAXoxtpXq8acKckbYeYDIgcxwB0Xrlgwagor8r6Y50mHK32wcsvTgObgyPO/tnD12jaNkL1VN9nB71N9QVJfs86mRQc1Ht48Tm2+IhbjkrWDBxGmnZH8/8oCuaowsRHb/+UKvUFgh/R87vnPobkKL/DMW0+hReyU27TKNwSOjsyE1P4yisZdb50gqEQm383z+lXbQSmKBz0NuZn86RQgnYJQ7Cp0Jhj5hKNb5vlaCLRXzYX6ZC64="
    }
}
