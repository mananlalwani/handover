package org.handover.android

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.MessageDigest
import java.security.cert.X509Certificate
import javax.security.auth.x500.X500Principal

/** Stable P-256 identity. Private key material remains in the Android Keystore. */
class DeviceIdentityStore(context: Context) {
    private val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }

    val deviceId: String
        get() = certificate().fingerprint()

    fun certificate(): X509Certificate {
        if (!keyStore.containsAlias(KEY_ALIAS)) generateKey()
        return keyStore.getCertificate(KEY_ALIAS) as X509Certificate
    }

    fun keyStore(): KeyStore = keyStore

    private fun generateKey() {
        val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, ANDROID_KEYSTORE)
        generator.initialize(KeyGenParameterSpec.Builder(
            KEY_ALIAS,
            KeyProperties.PURPOSE_SIGN
        ).setAlgorithmParameterSpec(java.security.spec.ECGenParameterSpec("secp256r1"))
            .setDigests(KeyProperties.DIGEST_NONE, KeyProperties.DIGEST_SHA256)
            .setCertificateSubject(X500Principal("CN=Handover Android"))
            .setCertificateSerialNumber(java.math.BigInteger.ONE)
            .setCertificateNotBefore(java.util.Date())
            .setCertificateNotAfter(java.util.Date(System.currentTimeMillis() + 10L * 365 * 24 * 60 * 60 * 1000))
            .build())
        generator.generateKeyPair()
    }

    companion object {
        private const val ANDROID_KEYSTORE = "AndroidKeyStore"
        private const val KEY_ALIAS = "handover-native-p256-v2"

        fun X509Certificate.fingerprint(): String = MessageDigest.getInstance("SHA-256")
            .digest(encoded).joinToString("") { "%02x".format(it) }
    }
}
