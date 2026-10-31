package example.hq.spike.update

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class UpdateLogicTest {
    private val sha = "a".repeat(64)
    private val url = "${UpdateLogic.APK_URL_PREFIX}v0.9.1-main.7/agent-hq-android.apk"

    private fun json(
        version: String = "0.9.1-main.7", code: String = "7", url: String = this.url, sha: String = this.sha, minSdk: String = "26",
    ) = """{"version":"$version","versionCode":$code,"url":"$url","sha256":"$sha","minSdk":$minSdk}"""

    @Test fun parsesAGoodManifest() {
        val m = UpdateLogic.parse(json())
        assertEquals(ChannelManifest("0.9.1-main.7", 7, url, sha, 26), m)
    }

    @Test fun lowercasesTheHash() {
        assertEquals(sha, UpdateLogic.parse(json(sha = sha.uppercase()))?.sha256)
    }

    @Test fun refusesAUrlOutsideTheReleasePath() {
        assertNull(UpdateLogic.parse(json(url = "https://example.com/agent-hq-android.apk")))
        assertNull(UpdateLogic.parse(json(url = "http://github.com/CalvinMagezi/hq/releases/download/v1/a.apk")))
        assertNull(UpdateLogic.parse(json(url = UpdateLogic.APK_URL_PREFIX + "v1/payload.zip")))
    }

    @Test fun refusesABadHashCodeOrSdk() {
        assertNull(UpdateLogic.parse(json(sha = "abc")))
        assertNull(UpdateLogic.parse(json(sha = "g".repeat(64))))
        assertNull(UpdateLogic.parse(json(code = "0")))
        assertNull(UpdateLogic.parse(json(minSdk = "0")))
    }

    @Test fun refusesMissingFieldsAndNonJson() {
        assertNull(UpdateLogic.parse("""{"version":"1"}"""))
        assertNull(UpdateLogic.parse("not json"))
        assertNull(UpdateLogic.parse(""))
    }

    @Test fun aBuildIsNewerOnlyByVersionCode() {
        val m = UpdateLogic.parse(json(code = "8"))!!
        assertEquals(UpdateVerdict.UpToDate, UpdateLogic.verdict(m, 8, 35))
        assertEquals(UpdateVerdict.UpToDate, UpdateLogic.verdict(m, 9, 35))
        assertEquals(UpdateVerdict.Available(m), UpdateLogic.verdict(m, 7, 35))
    }

    @Test fun anOlderAndroidIsToldWhy() {
        val m = UpdateLogic.parse(json(code = "8", minSdk = "30"))!!
        assertEquals(UpdateVerdict.NeedsNewerAndroid(30), UpdateLogic.verdict(m, 7, 28))
        assertEquals(UpdateVerdict.UpToDate, UpdateLogic.verdict(m, 8, 28))
    }

    @Test fun dueOncePerDay() {
        val day = UpdateLogic.CHECK_EVERY_MS
        assertTrue(UpdateLogic.due(0, 1000))
        assertFalse(UpdateLogic.due(1000, 1000 + day - 1))
        assertTrue(UpdateLogic.due(1000, 1000 + day))
        assertTrue(UpdateLogic.due(5000, 1000))
    }

    @Test fun stableFollowsLatestAndMainItsOwnRelease() {
        assertEquals("https://github.com/CalvinMagezi/hq/releases/latest/download/android-channel.json", UpdateLogic.manifestUrl(UpdateChannel.Stable))
        assertEquals("${UpdateLogic.APK_URL_PREFIX}channel-main/android-channel.json", UpdateLogic.manifestUrl(UpdateChannel.Main))
        assertEquals(UpdateChannel.Stable, UpdateChannel.of(null))
        assertEquals(UpdateChannel.Stable, UpdateChannel.of("nightly"))
        assertEquals(UpdateChannel.Main, UpdateChannel.of("main"))
    }

    @Test fun everyStateHasWords() {
        val m = UpdateLogic.parse(json())!!
        val states = listOf(UpdateState.Idle, UpdateState.Checking, UpdateState.UpToDate, UpdateState.Available(m), UpdateState.NeedsNewerAndroid(30), UpdateState.Downloading, UpdateState.Installing, UpdateState.Failed("No."))
        states.forEach { assertTrue(UpdateLogic.words(it).isNotBlank()) }
        assertNotNull(UpdateLogic.words(UpdateState.Available(m)).takeIf { "0.9.1-main.7" in it })
    }
}
