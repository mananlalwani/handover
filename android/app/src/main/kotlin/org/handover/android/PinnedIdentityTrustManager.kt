package org.handover.android

import android.annotation.SuppressLint
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import java.security.interfaces.ECPublicKey
import javax.net.ssl.X509TrustManager
import org.handover.android.DeviceIdentityStore.Companion.fingerprint

/** TLS policy for the native pairing transport.
 *
 * Before pairing, the certificate is authenticated by the explicit comparison
 * code ceremony, so there is intentionally no public-CA requirement. The
 * certificate still must be present and valid. Once paired, only its exact
 * certificate fingerprint is trusted.
 */
@SuppressLint("CustomX509TrustManager")
internal class PinnedIdentityTrustManager(private val pin: String?) : X509TrustManager {
    override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()

    override fun checkClientTrusted(chain: Array<X509Certificate>, authType: String) {
        validate(chain)
    }

    override fun checkServerTrusted(chain: Array<X509Certificate>, authType: String) {
        val leaf = validate(chain)
        if (pin != null && leaf.fingerprint() != pin) {
            throw CertificateException("server certificate pin mismatch")
        }
    }

    private fun validate(chain: Array<X509Certificate>): X509Certificate {
        val leaf = chain.firstOrNull() ?: throw CertificateException("empty certificate chain")
        runCatching { leaf.checkValidity() }
            .getOrElse { throw CertificateException("certificate is not currently valid", it) }
        val key = leaf.publicKey as? ECPublicKey
            ?: throw CertificateException("identity certificate must use an EC public key")
        val params = key.params
        if (params.curve.field.fieldSize != 256 || params.order.bitLength() != 256) {
            throw CertificateException("identity certificate must use P-256")
        }
        if (leaf.subjectX500Principal != leaf.issuerX500Principal) {
            throw CertificateException("identity certificate must be self-signed")
        }
        runCatching { leaf.verify(key) }
            .getOrElse { throw CertificateException("identity certificate signature is invalid", it) }
        return leaf
    }
}
