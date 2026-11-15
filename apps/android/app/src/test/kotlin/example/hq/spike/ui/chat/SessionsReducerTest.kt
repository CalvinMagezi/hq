package example.hq.spike.ui.chat

import example.hq.spike.data.WatchedSession
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SessionsReducerTest {
    private fun session(id: String, drive: Boolean = false, status: String = "working", running: Boolean = true, wake: Boolean = false, label: String = "") =
        WatchedSession(id, label, "claude-code", status, running, drive, wake, "T-1")

    private val two = SessionsUi(sessions = listOf(session("a"), session("b", status = "blocked")))

    @Test fun countsWatchedSessionsAndThoseWaitingOnAnAnswer() {
        assertEquals(2, two.count)
        assertEquals(1, two.waiting)
        assertEquals(0, SessionsUi().count)
    }

    @Test fun aWakeNotYetHandledCountsAsWaitingButOnlyWhileRunning() {
        val ui = SessionsUi(sessions = listOf(session("a", wake = true), session("b", wake = true, running = false)))
        assertEquals(1, ui.waiting)
    }

    @Test fun theSwitchFlipsAtOnceAndTheRowIsBusy() {
        val out = SessionsReducer.driveRequested(two, "a", true)
        assertTrue(out.sessions.first { it.id == "a" }.drive)
        assertEquals(setOf("a"), out.busy)
    }

    @Test fun aSecondTapWhileBusyChangesNothing() {
        val once = SessionsReducer.driveRequested(two, "a", true)
        assertEquals(once, SessionsReducer.driveRequested(once, "a", false))
    }

    @Test fun aRefusedRequestPutsTheSwitchBack() {
        val out = SessionsReducer.driveFailed(SessionsReducer.driveRequested(two, "a", true), "a", was = false)
        assertFalse(out.sessions.first { it.id == "a" }.drive)
        assertTrue(out.busy.isEmpty())
    }

    @Test fun theSavedRowReplacesTheLocalOne() {
        val out = SessionsReducer.driveSaved(SessionsReducer.driveRequested(two, "a", true), session("a", drive = true, status = "idle"))
        assertEquals("idle", out.sessions.first { it.id == "a" }.agentStatus)
        assertTrue(out.busy.isEmpty())
    }

    @Test fun aRefreshKeepsTheLocalSwitchOfARowStillInFlight() {
        val flying = SessionsReducer.driveRequested(two, "a", true)
        val out = SessionsReducer.loaded(flying, listOf(session("a", drive = false), session("b")))
        assertTrue(out.sessions.first { it.id == "a" }.drive)
        assertEquals(setOf("a"), out.busy)
    }

    @Test fun aRefreshDropsBusyForARowThatIsGone() {
        val flying = SessionsReducer.driveRequested(two, "a", true)
        val out = SessionsReducer.loaded(flying, listOf(session("b")))
        assertEquals(listOf("b"), out.sessions.map { it.id })
        assertTrue(out.busy.isEmpty())
    }

    @Test fun stoppingWatchingRemovesTheRowAndAnEmptyListIsEmpty() {
        val out = SessionsReducer.stopped(SessionsReducer.stopped(SessionsReducer.stopRequested(two, "a"), "a"), "b")
        assertTrue(out.sessions.isEmpty())
        assertEquals(0, out.count)
        assertTrue(out.busy.isEmpty())
    }

    @Test fun aFailedStopKeepsTheRow() {
        val out = SessionsReducer.stopFailed(SessionsReducer.stopRequested(two, "a"), "a")
        assertEquals(2, out.count)
        assertTrue(out.busy.isEmpty())
    }

    @Test fun theNameFallsBackFromLabelToHarnessToAPlainWord() {
        assertEquals("Build", SessionsReducer.name(session("a", label = "Build"), "Agent"))
        assertEquals("claude-code", SessionsReducer.name(session("a"), "Agent"))
        assertEquals("Agent", SessionsReducer.name(session("a").copy(harness = ""), "Agent"))
    }
}
