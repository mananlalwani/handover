package org.handover.android

/** No service codes, URI syntax, pauses or extensions. Emergency checking is
 * performed separately by Android's telephony service immediately before dial. */
fun validCallAddress(address: String): Boolean {
    val digits = address.removePrefix("+")
    return digits.length in 1..15 && digits.all { it in '0'..'9' }
}
