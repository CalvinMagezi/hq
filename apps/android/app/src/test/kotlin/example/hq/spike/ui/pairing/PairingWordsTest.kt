package example.hq.spike.ui.pairing

import example.hq.spike.bridge.BridgePeer
import example.hq.spike.bridge.BridgeState
import example.hq.spike.bridge.TicketEnd
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class PairingWordsTest {
    @Test fun theSwitchSaysWhatIsListening() {
        assertEquals("Off. Nothing can reach this phone.", PairingWords.switchLine(BridgeState()))
        assertEquals("Starting…", PairingWords.switchLine(BridgeState(enabled = true)))
        assertEquals("On. Paired devices can reach this phone as 66902dde.", PairingWords.switchLine(BridgeState(enabled = true, running = true, deviceId = "66902ddec14cff33")))
    }

    @Test fun theRelayLineAdmitsTheDevelopmentDefault() {
        assertTrue(PairingWords.relayLine("default")!!.contains("development"))
        assertEquals(null, PairingWords.relayLine(null))
    }

    @Test fun theCountdownRoundsUpAndNeverGoesNegative() {
        assertEquals("10:00", PairingWords.countdown(600_000))
        assertEquals("9:41", PairingWords.countdown(581_000 - 1))
        assertEquals("0:01", PairingWords.countdown(1))
        assertEquals("0:00", PairingWords.countdown(-5_000))
    }

    @Test fun theDigitFieldTakesSixDigitsAndNothingElse() {
        assertEquals("123456", PairingWords.typed("12 34-56 789"))
        assertEquals("", PairingWords.typed("abc"))
    }

    @Test fun theRequestNamesTheDeviceWithItsShortIdAndAsksForDigits() {
        val text = PairingWords.wants("Pixel 8", "ab12cd34ef56")
        assertTrue(text, text.startsWith("“Pixel 8” (ab12cd34) wants to pair."))
        assertTrue(text.endsWith("Type the six digits it shows."))
    }

    @Test fun everyWayATicketEndsHasWords() {
        TicketEnd.entries.forEach { assertTrue(PairingWords.ended(it).isNotBlank()) }
    }

    @Test fun lastSeenIsInTheUnitsAPersonUses() {
        assertEquals("just now", PairingWords.ago(5))
        assertEquals("5 min ago", PairingWords.ago(300))
        assertEquals("3 h ago", PairingWords.ago(3 * 3600))
        assertEquals("2 d ago", PairingWords.ago(2 * 86_400))
        assertEquals("not seen yet", PairingWords.peerLine(BridgePeer("aabbccdd11", "x", 1, null, null), 100).substringAfter("· "))
    }

    @Test fun theBatteryNoticeShowsOnlyForAPhoneThatIsOnAndPausable() {
        assertEquals(false, PairingWords.showBatteryNotice(BridgeState(), false))
        assertEquals(false, PairingWords.showBatteryNotice(BridgeState(enabled = true), true))
        assertEquals(true, PairingWords.showBatteryNotice(BridgeState(enabled = true), false))
    }
}
