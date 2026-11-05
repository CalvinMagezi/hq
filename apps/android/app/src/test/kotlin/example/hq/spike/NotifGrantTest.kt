package example.hq.spike

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import example.hq.spike.measure.Counter
import example.hq.spike.measure.Counters
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class NotifGrantTest {
    private val c: Context get() = ApplicationProvider.getApplicationContext()

    @Test fun anAnswerIsRememberedAndTheRightCounterMoves() {
        assertFalse(NotifAccess.read(c).asked)
        val counters = Counters.get(c)
        val before = listOf(Counter.NotificationsAllowedFirstRun, Counter.NotificationsAllowedLater, Counter.NotificationsNotAllowed).map { counters.snapshot().current.slot(it) }
        NotifAccess.answered(c, NotifOrigin.FirstRun, granted = true, rationale = false)
        NotifAccess.answered(c, NotifOrigin.Later, granted = true, rationale = false)
        NotifAccess.answered(c, NotifOrigin.Later, granted = false, rationale = true)
        assertEquals(before[0] + 1, counters.snapshot().current.slot(Counter.NotificationsAllowedFirstRun))
        assertEquals(before[1] + 1, counters.snapshot().current.slot(Counter.NotificationsAllowedLater))
        assertEquals(before[2] + 1, counters.snapshot().current.slot(Counter.NotificationsNotAllowed))
        val f = NotifAccess.read(c)
        assertTrue(f.asked)
        assertTrue(f.rationale)
    }

    @Test fun theSnoozeLastsSevenDaysAndIsClearedWhenItEnds() {
        NotifAccess.snooze(c, now = 1_000L)
        assertEquals(1_000L + NotifAccess.SNOOZE_MS, NotifAccess.snoozeUntil(c))
        NotifAccess.clearSnooze(c)
        assertEquals(0L, NotifAccess.snoozeUntil(c))
    }

    @Test fun updatesFoldAfterThreeAnHour() {
        assertEquals(3, AlertPoster.updatesLeft(c, now = 10L))
    }
}
