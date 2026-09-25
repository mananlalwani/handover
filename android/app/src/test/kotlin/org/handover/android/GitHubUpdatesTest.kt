package org.handover.android

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.IOException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class GitHubUpdatesTest {
    private val digest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"

    @Test fun releaseSelectsOnlyTheVersionedSignedApk() {
        val release = parseGitHubRelease("""
            {"tag_name":"v0.3.3","draft":false,"prerelease":false,"assets":[
              {"id":1,"name":"handover-android-0.3.3-debug.apk","size":3,
               "state":"uploaded","digest":"sha256:$digest"},
              {"id":42,"name":"handover-android-0.3.3.apk","size":3,
               "state":"uploaded","digest":"sha256:$digest"}
            ]}
        """.trimIndent())
        assertEquals("0.3.3", release.version)
        assertEquals(42L, release.apk?.id)
        assertEquals(digest, release.apk?.sha256)
    }

    @Test fun missingDigestAndPrereleaseCannotSupplyAnUpdate() {
        val noDigest = parseGitHubRelease("""
            {"tag_name":"v0.3.3","draft":false,"prerelease":false,"assets":[
              {"id":42,"name":"handover-android-0.3.3.apk","size":3,"state":"uploaded"}
            ]}
        """.trimIndent())
        assertNull(noDigest.apk)
        val prerelease = """{"tag_name":"v0.3.3","draft":false,"prerelease":true,"assets":[]}"""
        try {
            parseGitHubRelease(prerelease)
            throw AssertionError("prerelease accepted")
        } catch (_: IOException) {
            // Only full published releases are update sources.
        }
    }

    @Test fun versionComparisonUsesNumbers() {
        assertTrue(isNewerRelease("0.3.10", "0.3.9"))
        assertFalse(isNewerRelease("0.3.2", "0.3.2"))
        assertFalse(isNewerRelease("0.3.2", "0.3.3"))
        assertFalse(isNewerRelease("0.3.3", "local-build"))
    }

    @Test fun downloadMustMatchSizeAndDigest() {
        val output = ByteArrayOutputStream()
        copyVerifiedApk(ByteArrayInputStream("abc".toByteArray()), output, 3, digest)
        assertEquals("abc", output.toString("UTF-8"))
        for (bytes in listOf("abd", "abcd", "ab")) {
            try {
                copyVerifiedApk(ByteArrayInputStream(bytes.toByteArray()), ByteArrayOutputStream(), 3, digest)
                throw AssertionError("accepted $bytes")
            } catch (_: IOException) {
                // Hash and size checks must reject incomplete or changed assets.
            }
        }
    }
}
