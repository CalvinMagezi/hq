package example.hq.spike

import example.hq.spike.ui.inbox.InboxBanner
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class NotifAccessTest {
    private fun facts(sdk: Int = 34, granted: Boolean = true, app: Boolean = true, needs: Int = 4, rationale: Boolean = false, asked: Boolean = true) =
        NotifFactsNow(sdk, granted, app, needs, rationale, asked)

    @Test fun android13NeverAskedAsksAndroid() {
        assertEquals(NotifStep.AskAndroid, NotifAccess.next(facts(granted = false, asked = false)))
    }

    @Test fun deniedWithARationaleAsksAgainAndWithoutOneOpensTheApp() {
        assertEquals(NotifStep.AskAndroid, NotifAccess.next(facts(granted = false, rationale = true)))
        assertEquals(NotifStep.OpenApp, NotifAccess.next(facts(granted = false, rationale = false)))
    }

    @Test fun android12AndOlderWithTheAppOffOpensTheApp() {
        assertEquals(NotifStep.OpenApp, NotifAccess.next(facts(sdk = 31, granted = true, app = false)))
        assertEquals(NotifStep.OpenApp, NotifAccess.next(facts(sdk = 28, app = false, asked = false)))
    }

    @Test fun theAppOnAndNeedsYouBlockedOpensTheChannel() {
        assertEquals(NotifStep.OpenChannel, NotifAccess.next(facts(needs = 0)))
    }

    @Test fun quieterNeedsYouAndEverythingOnAreDone() {
        assertEquals(NotifStep.Done, NotifAccess.next(facts(needs = 3)))
        assertEquals(NotifStep.Done, NotifAccess.next(facts(needs = 2)))
        assertEquals(NotifStep.Done, NotifAccess.next(facts()))
        assertTrue(facts(needs = 1).needsOn)
        assertFalse(facts(needs = 0).needsOn)
        assertFalse(facts(granted = false).needsOn)
    }

    @Test fun theUpgraderSheetShowsOnlyOnAndroid13AfterAnUpdateToSomeoneNeverAskedAndShowsOnce() {
        assertTrue(NotifAccess.showUpgrader(34, 3, 4, granted = false, asked = false, shownBefore = false))
        assertFalse(NotifAccess.showUpgrader(32, 3, 4, granted = false, asked = false, shownBefore = false))
        assertFalse(NotifAccess.showUpgrader(34, 4, 4, granted = false, asked = false, shownBefore = false))
        assertFalse(NotifAccess.showUpgrader(34, 0, 4, granted = false, asked = false, shownBefore = false))
        assertFalse(NotifAccess.showUpgrader(34, 3, 4, granted = true, asked = false, shownBefore = false))
        assertFalse(NotifAccess.showUpgrader(34, 3, 4, granted = false, asked = true, shownBefore = false))
        assertFalse(NotifAccess.showUpgrader(34, 3, 4, granted = false, asked = false, shownBefore = true))
    }

    @Test fun theBannerShowsWhileOffAndNotWhileSnoozedForSevenDays() {
        val off = facts(granted = false)
        val now = 1_000_000L
        assertEquals(InboxBanner(2, 1), NotifAccess.banner(off, 0, now, 2 to 1))
        assertNull(NotifAccess.banner(off, now + NotifAccess.SNOOZE_MS, now, 2 to 1))
        assertEquals(InboxBanner(0, 0), NotifAccess.banner(off, now, now, 0 to 0))
        assertNull(NotifAccess.banner(facts(), 0, now, 2 to 1))
        assertEquals(7L * 24 * 60 * 60 * 1000, NotifAccess.SNOOZE_MS)
    }

    @Test fun storedIdsSplitOnNewlinesAndSkipBlanks() {
        assertEquals(listOf("a", "b"), NotifAccess.ids("a\nb\n"))
        assertEquals(emptyList<String>(), NotifAccess.ids(null))
    }
}
