package example.hq.spike.voice

import android.media.AudioManager
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MicAskTest {
    private fun o(g: Boolean, b: Boolean, a: Boolean, ms: Long) = MicAsk.outcome(g, b, a, ms)

    @Test fun allowedListens() = assertEquals(MicOutcome.Granted, o(true, false, false, 900))
    @Test fun firstRefusalIsNeeded() = assertEquals(MicOutcome.Needed, o(false, false, true, 900))
    @Test fun secondRefusalIsBlocked() = assertEquals(MicOutcome.Blocked, o(false, true, false, 900))
    @Test fun aDismissedDialogChangesNothing() = assertEquals(MicOutcome.Dismissed, o(false, false, false, 900))
    @Test fun androidAnsweringByItselfIsBlocked() = assertEquals(MicOutcome.Blocked, o(false, false, false, 20))
    @Test fun stillNeededWhenRationaleStays() = assertEquals(MicOutcome.Needed, o(false, true, true, 900))

    @Test fun aCallBlocksTheMic() {
        assertTrue(MicAsk.onCall(AudioManager.MODE_IN_CALL))
        assertTrue(MicAsk.onCall(AudioManager.MODE_IN_COMMUNICATION))
        assertFalse(MicAsk.onCall(AudioManager.MODE_NORMAL))
    }

    @Test fun twoSilencesAddTheHint() {
        assertEquals(SilenceLine.Short, MicAsk.silenceLine(1, 35))
        assertEquals(SilenceLine.SwitchHint, MicAsk.silenceLine(2, 31))
        assertEquals(SilenceLine.SpeakCloser, MicAsk.silenceLine(2, 30))
    }

    @Test fun firstMicTapShowsReasonOnlyInReadyIdleOrStoppedPhases() {
        for (phase in listOf(VoicePhase.ModelReady, VoicePhase.Idle, VoicePhase.Stopped)) {
            assertTrue(MicAsk.shouldShowReason(granted = false, askedBefore = false, phase = phase))
            assertFalse(MicAsk.shouldShowReason(granted = true, askedBefore = false, phase = phase))
            assertFalse(MicAsk.shouldShowReason(granted = false, askedBefore = true, phase = phase))
            assertTrue(MicAsk.canLaunchDialog(granted = false, askedBefore = true, phase = phase))
        }
        for (busyPhase in listOf(VoicePhase.NeedsModel, VoicePhase.Downloading, VoicePhase.Preparing, VoicePhase.MicBlocked)) {
            assertFalse(MicAsk.shouldShowReason(granted = false, askedBefore = false, phase = busyPhase))
            assertFalse(MicAsk.canLaunchDialog(granted = false, askedBefore = false, phase = busyPhase))
            assertFalse(MicAsk.canLaunchDialog(granted = false, askedBefore = true, phase = busyPhase))
        }
        assertFalse(MicAsk.shouldShowReason(granted = false, askedBefore = false, phase = VoicePhase.MicNeeded))
        assertTrue(MicAsk.canLaunchDialog(granted = false, askedBefore = false, phase = VoicePhase.MicNeeded))
    }
}
