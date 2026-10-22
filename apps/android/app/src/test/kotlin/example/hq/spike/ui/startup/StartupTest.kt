package example.hq.spike.ui.startup

import example.hq.spike.data.HqStatus
import java.time.ZoneOffset
import org.junit.Assert.assertEquals
import org.junit.Test

class StartupTest {
    @Test fun theFirstStartGetsItsOwnScreenAndLaterOnesKeepTheApp() {
        assertEquals(Startup.Starting, StartupWords.decide(HqStatus.Starting(12), firstLaunchDone = false))
        assertEquals(Startup.None, StartupWords.decide(HqStatus.Starting(12), firstLaunchDone = true))
        assertEquals(Startup.None, StartupWords.decide(HqStatus.Live, firstLaunchDone = false))
        assertEquals(Startup.None, StartupWords.decide(HqStatus.Saved("1m"), firstLaunchDone = false))
    }

    @Test fun aMinuteWithoutAnAnswerIsAFailureOnEveryLaunch() {
        assertEquals(Startup.DidNotStart, StartupWords.decide(HqStatus.Starting(60), firstLaunchDone = false))
        assertEquals(Startup.DidNotStart, StartupWords.decide(HqStatus.Starting(95), firstLaunchDone = true))
        assertEquals(Startup.Starting, StartupWords.decide(HqStatus.Starting(59), firstLaunchDone = false))
    }

    @Test fun theBoxSaysWhenHqLastStartedAndHowItEnded() {
        val at = java.time.Instant.parse("2026-10-22T14:31:00Z").toEpochMilli()
        assertEquals(listOf("Health check timed out after 60 s.", "Last start: 14:31 \u00b7 exit code none"), StartupWords.failedLines(at, null, ZoneOffset.UTC))
        assertEquals("Last start: 14:31 \u00b7 exit code 137", StartupWords.failedLines(at, 137, ZoneOffset.UTC)[1])
        assertEquals("Last start: never \u00b7 exit code none", StartupWords.failedLines(0, null)[1])
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
