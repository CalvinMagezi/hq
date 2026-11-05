package example.hq.spike.ui.settings

import androidx.test.core.app.ApplicationProvider
import example.hq.spike.ui.shell.Words
import example.hq.spike.voice.MicFact
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class AssistantWordsTest {
    private val w = Words.of(ApplicationProvider.getApplicationContext())
    private fun s(a: Boolean, h: Boolean, m: MicFact, r: Boolean) = AssistantWords.status(w, a, h, m, r)

    @Test fun theFiveStatusesInOrder() {
        assertEquals("Not available on this phone", s(false, true, MicFact.Off, false))
        assertEquals("Not set up", s(true, false, MicFact.Off, false))
        assertEquals("On · microphone off", s(true, true, MicFact.Off, false))
        assertEquals("On · speech model not downloaded", s(true, true, MicFact.Allowed, false))
        assertEquals("On · hold Home to ask", s(true, true, MicFact.NotAsked, true))
    }

    @Test fun onlyTheMicrophoneWarns() {
        assertTrue(AssistantWords.warn(true, true, MicFact.Off))
        assertFalse(AssistantWords.warn(true, false, MicFact.Off))
        assertFalse(AssistantWords.warn(true, true, MicFact.Allowed))
    }
}
