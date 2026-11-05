package example.hq.spike.data

import example.hq.spike.ui.firstrun.KeyNames
import example.hq.spike.ui.shell.Words
import android.content.Context
import androidx.test.core.app.ApplicationProvider
import java.time.Instant
import java.time.ZoneId
import java.util.Locale
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class InstantsZoneTest {
    private val want = Instant.parse("2026-10-22T04:00:00Z")

    @Test fun everyShapeHQSendsReadsAsTheSameInstant() {
        listOf("2026-10-22 04:00:00", "2026-10-22T04:00:00", "2026-10-22T04:00:00Z", "2026-10-22T04:00:00+00:00", "2026-10-22 04:00:00Z")
            .forEach { assertEquals(it, want, Instants.parse(it)) }
        assertEquals(want.plusMillis(250), Instants.parse("2026-10-22T04:00:00.250Z"))
    }

    @Test fun badInputIsNull() {
        listOf(null, "", "  ", "yesterday", "2026-13-40 00:00:00").forEach { assertNull(it, Instants.parse(it)) }
    }

    @Test fun hqZoneReadsTheZoneAndNamesTheCity() {
        assertEquals("London", HqZone.parse("""{"tz":"Europe/London"}""")?.city)
        assertEquals("Buenos Aires", HqZone.parse("""{"tz":"America/Argentina/Buenos_Aires"}""")?.city)
        assertNull(HqZone.parse("""{"tz":"Not/AZone"}"""))
        assertNull(HqZone.parse("""{"tz_offset_minutes":0}"""))
        assertNull(HqZone.parse("not json"))
    }

    @Test fun keyNamesAreReadable() {
        assertEquals("OpenRouter", KeyNames.display("OPENROUTER_API_KEY"))
        assertEquals("SOME_OTHER_KEY", KeyNames.display("SOME_OTHER_KEY"))
    }

    @Test fun theClockFollowsTheTwelveOrTwentyFourHourSetting() {
        Locale.setDefault(Locale.US)
        val res = ApplicationProvider.getApplicationContext<Context>().resources
        val at = Instant.parse("2026-10-22T14:32:00Z")
        val utc = ZoneId.of("UTC")
        assertEquals("14:32", Words(res, true).clock(at, utc))
        assertEquals("2:32\u202FPM", Words(res, false).clock(at, utc).replace(' ', '\u202F'))
    }
}
