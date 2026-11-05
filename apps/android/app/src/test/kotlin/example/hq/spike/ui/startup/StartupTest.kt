package example.hq.spike.ui.startup

import example.hq.spike.testing.testWords

import example.hq.spike.data.Connection
import java.time.ZoneOffset
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class StartupTest {
    @org.junit.Before fun english() = java.util.Locale.setDefault(java.util.Locale.ENGLISH)

    @Test fun theFirstStartGetsItsOwnScreenAndLaterOnesKeepTheApp() {
        assertEquals(Startup.Starting, StartRules.decide(Connection.Starting(0, 12, null), firstLaunchDone = false))
        assertEquals(Startup.None, StartRules.decide(Connection.Starting(0, 12, null), firstLaunchDone = true))
        assertEquals(Startup.None, StartRules.decide(Connection.Connected, firstLaunchDone = false))
        assertEquals(Startup.None, StartRules.decide(Connection.Offline(0), firstLaunchDone = false))
    }

    @Test fun aSlowOrLongStartNeverBecomesACover() {
        assertEquals(Startup.None, StartRules.decide(Connection.Starting(0, 95, null), firstLaunchDone = true))
        assertEquals(Startup.None, StartRules.decide(Connection.Stopped(0, 2), firstLaunchDone = true))
        assertEquals(Startup.Starting, StartRules.decide(Connection.Starting(0, 59, null), firstLaunchDone = false))
    }

    @Test fun theSignedSwitchBringsTheOldCoverBack() {
        assertEquals(Startup.Starting, StartRules.decide(Connection.Starting(0, 12, null), firstLaunchDone = true, coverRemoval = false))
        assertEquals(Startup.None, StartRules.decide(Connection.Starting(0, 12, null), firstLaunchDone = true, coverRemoval = true))
    }

    @Test fun theStepComesFromWhatThePhoneCanSee() {
        assertEquals(1, StartRules.step(hubUp = false, alive = false, portOpen = false))
        assertEquals(2, StartRules.step(hubUp = true, alive = false, portOpen = false))
        assertEquals(3, StartRules.step(hubUp = true, alive = true, portOpen = false))
        assertEquals(4, StartRules.step(hubUp = true, alive = true, portOpen = true))
    }

    @Test fun usualIsTheMedianOfTheLastFiveRoundedToFiveSecondsFromTheSecondStart() {
        assertEquals(null, StartRules.usual(emptyList()))
        assertEquals(null, StartRules.usual(listOf(20_000L)))
        assertEquals(25L, StartRules.usual(listOf(22_000L, 27_000L, 90_000L, 24_000L, 26_000L)))
        assertEquals(25L, StartRules.usual(listOf(1_000_000L, 22_000L, 27_000L, 90_000L, 24_000L, 26_000L)))
        assertEquals(5L, StartRules.usual(listOf(300L, 500L)))
    }

    @Test fun slowIsPastTwiceTheUsualAndNeverBeforeThirtySeconds() {
        assertEquals(false, StartRules.slow(72, null))
        assertEquals(false, StartRules.slow(25, 5))
        assertEquals(true, StartRules.slow(31, 5))
        assertEquals(false, StartRules.slow(72, 40))
        assertEquals(true, StartRules.slow(81, 40))
    }

    @Test fun aLiveStartPastAMinuteInStepThreeAsksBeforeRestarting() {
        assertEquals(true, StartRules.stillOpening(60, 3))
        assertEquals(false, StartRules.stillOpening(59, 3))
        assertEquals(false, StartRules.stillOpening(90, 2))
    }

    @Test fun underTwoHundredMegabytesIsAlmostFull() {
        assertEquals(true, StartRules.almostFull(199_999_999L))
        assertEquals(false, StartRules.almostFull(200_000_000L))
        assertEquals(false, StartRules.almostFull(null))
    }

    @Test fun theStoppedSheetOpensByItselfAtMostOnceADay() {
        val day = 24 * 60 * 60 * 1000L
        assertEquals(true, StartRules.autoOpenOk(0, 5))
        assertEquals(false, StartRules.autoOpenOk(1_000, 1_000 + day - 1))
        assertEquals(true, StartRules.autoOpenOk(1_000, 1_000 + day))
    }

    @Test fun onlyTheLastFiveStartsAreKept() {
        val c = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.content.Context>()
        (1..7).forEach { StartRules.recordStart(c, it * 1000L) }
        assertEquals(listOf(3000L, 4000L, 5000L, 6000L, 7000L), StartRules.starts(c))
    }

    @Test fun theBoxSaysWhenHqLastStartedAndHowItEnded() {
        val at = java.time.Instant.parse("2026-10-22T14:31:00Z").toEpochMilli()
        assertEquals(listOf("Health check timed out after 60 s.", "Last start 2:31 PM \u00b7 exit code none"), StartupWords.failedLines(testWords(), at, null, 60, ZoneOffset.UTC))
        assertEquals("Last start 2:31 PM \u00b7 exit code 137", StartupWords.failedLines(testWords(), at, 137, 60, ZoneOffset.UTC)[1])
        assertEquals("Last start none \u00b7 exit code none", StartupWords.failedLines(testWords(), 0, null, 60)[1])
    }

    @Test fun aCopiedReportKeepsTheEndOfTheLogWithTokensBlankedAndLinesCut() {
        val token = "a".repeat(40)
        val lines = (1..20).map { "line $it" } + "HQ_WEB_AUTH_TOKEN=$token" + "x".repeat(500)
        val clean = StartupWords.cleanLog(lines, keep = 3)
        assertEquals(3, clean.size)
        assertEquals("HQ_WEB_AUTH_TOKEN=\u2026", clean[1])
        assertEquals(true, clean[2].length <= 1)
    }

    @Test fun theDetailsNameTheBuildTheModeAndTheLog() {
        val text = StartupWords.details("hub \u00b7 build 1", true, listOf("a", "b"), listOf("log1"))
        assertEquals("AgentHQ hub \u00b7 build 1\nRun mode: Active\na\nb\n\nRecent hq log:\nlog1\n", text)
        assertEquals("AgentHQ x\nRun mode: Standby\n", StartupWords.details("x", false, emptyList(), emptyList()))
    }
}
