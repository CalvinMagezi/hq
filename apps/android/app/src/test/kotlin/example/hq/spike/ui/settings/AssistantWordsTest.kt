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

    @Test fun theMicrophoneCanAlwaysBeAskedForAgain() {
        assertEquals(MicAction.None, AssistantWords.micAction(MicFact.Allowed, blocked = false))
        assertEquals(MicAction.Allow, AssistantWords.micAction(MicFact.NotAsked, blocked = false))
        assertEquals(MicAction.AskAgain, AssistantWords.micAction(MicFact.Off, blocked = false))
        // Android answered without its dialog: App info is the only way left.
        assertEquals(MicAction.AppInfo, AssistantWords.micAction(MicFact.Off, blocked = true))
        assertEquals(MicAction.None, AssistantWords.micAction(MicFact.Allowed, blocked = true))
    }

    @Test fun comingBackFromAndroidSettingsSaysWhetherItWorked() {
        assertEquals(null, AssistantWords.afterSettings(wentToSettings = false, held = false))
        assertEquals("HQ is now your phone's assistant. Hold Home on any screen to ask.", w.s(AssistantWords.afterSettings(true, true)!!))
        assertEquals("HQ is not the assistant yet. In Android's list, choose AgentHQ, then tap OK.", w.s(AssistantWords.afterSettings(true, false)!!))
    }

    @Test fun onlyTheMicrophoneWarns() {
        assertTrue(AssistantWords.warn(true, true, MicFact.Off))
        assertFalse(AssistantWords.warn(true, false, MicFact.Off))
        assertFalse(AssistantWords.warn(true, true, MicFact.Allowed))
    }
}
