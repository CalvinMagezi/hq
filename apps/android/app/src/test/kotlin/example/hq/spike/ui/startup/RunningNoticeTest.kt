package example.hq.spike.ui.startup

import example.hq.spike.data.HqStatus
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
        val active = RunningNotice.of(w, HqStatus.Live, active = true)
        assertEquals("HQ is running", active.title)
        assertEquals("Answering your chats and running scheduled tasks.", active.body)
        assertEquals("Only the app is on. Chats and scheduled tasks are paused.", RunningNotice.of(w, HqStatus.Live, active = false).body)
        assertEquals("Only the app is on. Your server HQ answers your chats.", RunningNotice.of(w, HqStatus.Live, active = false, serverAnswers = true).body)
    }

    @Test fun startingNamesTheStepAndNeverTheSeconds() {
        val a = RunningNotice.of(w, HqStatus.Starting(12, 3, null), true)
        assertEquals("HQ is starting", a.title)
        assertEquals("Step 3 of 4 \u00b7 Opening your notes and tasks", a.body)
        assertEquals(RunningNotice.key(w, HqStatus.Starting(12, 3, null), true), RunningNotice.key(w, HqStatus.Starting(40, 3, 25), true))
        assertFalse(RunningNotice.key(w, HqStatus.Starting(12, 3, null), true) == RunningNotice.key(w, HqStatus.Starting(12, 4, null), true))
    }

    @Test fun restartingAndStoppedHaveTheirOwnWordsAndOnlyStoppedOpensTheSheet() {
        val r = RunningNotice.of(w, HqStatus.Restarting(2), true)
        assertEquals("HQ is restarting", r.title)
        assertEquals("A setting changed. It is back in a few seconds.", r.body)
        assertFalse(r.opensSheet)
        val s = RunningNotice.of(w, HqStatus.Stopped(0, 3), true)
        assertEquals("HQ has stopped", s.title)
        assertEquals("Tap to see why.", s.body)
        assertTrue(s.opensSheet)
    }
}
