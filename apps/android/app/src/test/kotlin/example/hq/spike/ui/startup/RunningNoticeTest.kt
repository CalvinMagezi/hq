package example.hq.spike.ui.startup

import example.hq.spike.data.Connection
import example.hq.spike.testing.testWords
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class RunningNoticeTest {
    private val w = testWords()

    @Test fun runningSaysWhatTheModeDoes() {
        val active = RunningNotice.of(w, Connection.Connected, active = true)
        assertEquals("HQ is running", active.title)
        assertEquals("Answering your chats and running scheduled tasks.", active.body)
        assertEquals("Only the app is on. Chats and scheduled tasks are paused.", RunningNotice.of(w, Connection.Connected, active = false).body)
        assertEquals("Only the app is on. Your server HQ answers your chats.", RunningNotice.of(w, Connection.Connected, active = false, serverAnswers = true).body)
    }

    @Test fun startingNamesTheStepAndNeverTheSeconds() {
        val a = RunningNotice.of(w, Connection.Starting(2, 12, null), true)
        assertEquals("HQ is starting", a.title)
        assertEquals("Step 3 of 4 \u00b7 Opening your Vault and tasks", a.body)
        assertEquals(RunningNotice.key(w, Connection.Starting(2, 12, null), true), RunningNotice.key(w, Connection.Starting(2, 40, 25L), true))
        assertFalse(RunningNotice.key(w, Connection.Starting(2, 12, null), true) == RunningNotice.key(w, Connection.Starting(3, 12, null), true))
    }

    @Test fun restartingAndStoppedHaveTheirOwnWordsAndOnlyStoppedOpensTheSheet() {
        val r = RunningNotice.of(w, Connection.Restarting(2), true)
        assertEquals("HQ is restarting", r.title)
        assertEquals("A setting changed. It is back in a few seconds.", r.body)
        assertFalse(r.opensSheet)
        val s = RunningNotice.of(w, Connection.Stopped(0, 2), true)
        assertEquals("HQ has stopped", s.title)
        assertEquals("Tap to see why.", s.body)
        assertTrue(s.opensSheet)
    }
}
