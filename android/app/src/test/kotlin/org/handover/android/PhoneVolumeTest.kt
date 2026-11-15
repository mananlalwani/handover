package org.handover.android

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PhoneVolumeTest {
    private fun request(level: Any, kind: Any = "media", id: Any = "0123456789abcdef0123456789abcdef") =
        JSONObject().put("request_id", id).put("kind", kind).put("level", level)

    @Test fun acceptsOnlyKnownStreamAndIntegralLevelWithRequestId() {
        val parsed = parsePhoneVolumeRequest(request(4))
        assertEquals("media", parsed?.kind)
        assertEquals(4, parsed?.level)
        assertNull(parsePhoneVolumeRequest(request(4, "master")))
        assertNull(parsePhoneVolumeRequest(request(4, "MEDIA")))
        assertNull(parsePhoneVolumeRequest(request(4, id = "bad")))
    }

    @Test fun rejectsFractionalBooleanAndStringLevels() {
        assertNull(parsePhoneVolumeRequest(request(2.5)))
        assertNull(parsePhoneVolumeRequest(request(true)))
        assertNull(parsePhoneVolumeRequest(request("3")))
        assertNull(parsePhoneVolumeRequest(request(Long.MAX_VALUE)))
    }
}
