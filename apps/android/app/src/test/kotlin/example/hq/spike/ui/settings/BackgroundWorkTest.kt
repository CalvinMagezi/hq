package example.hq.spike.ui.settings

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class BackgroundWorkTest {
    private val health = JSONObject("""{"status":"ok"}""")
    private fun chat(telegram: String, discord: String = """{"enabled":false,"token_set":false}""") = JSONObject("""{"telegram":$telegram,"discord":$discord}""")
    private val telegramOn = """{"enabled":true,"token_set":true,"relay":{"status":"connected","current":true}}"""

    @Test fun hqIsRunningWhenItAnswersAndRestartingWhileItIs() {
        val snap = SettingsSnapshot(health = health, chatApps = chat(telegramOn))
        assertEquals(FactRow("HQ", "Running", good = true), SettingsWords.rightNow(snap, true, false)[0])
        assertEquals(FactRow("HQ", "Restarting"), SettingsWords.rightNow(snap, true, true)[0])
        assertEquals(FactRow("HQ", "Not answering"), SettingsWords.rightNow(SettingsSnapshot(), true, false)[0])
    }

    @Test fun aChatAppIsConnectedOnlyWhenSetUpActiveAndTheRelayIsCurrent() {
        val snap = SettingsSnapshot(health = health, chatApps = chat(telegramOn))
        assertEquals(listOf("Connected", "Not set up"), SettingsWords.rightNow(snap, true, false).drop(1).map { it.value })
        assertEquals(listOf("Paused", "Not set up"), SettingsWords.rightNow(snap, false, false).drop(1).map { it.value })
    }

    @Test fun aRelayFromAnEarlierRunIsNotConnectedYet() {
        val old = chat("""{"enabled":true,"token_set":true,"relay":{"status":"connected","current":false}}""")
        assertEquals("Not connected yet", SettingsWords.rightNow(SettingsSnapshot(health = health, chatApps = old), true, false)[1].value)
    }

    @Test fun aRelayThatFailedSaysError() {
        val bad = chat("""{"enabled":true,"token_set":true,"relay":{"status":"error","current":true}}""")
        assertEquals("Error", SettingsWords.rightNow(SettingsSnapshot(health = health, chatApps = bad), true, false)[1].value)
    }

    @Test fun onlyRunningIsGreen() {
        val rows = SettingsWords.rightNow(SettingsSnapshot(health = health, chatApps = chat(telegramOn)), true, false)
        assertEquals(listOf(true, false, false), rows.map { it.good })
    }

    @Test fun theDialogNamesWhatStopsAndWhatStarts() {
        assertEquals(SettingsWords.ModeWords("Pause background work?", "HQ will stop answering your chats and running scheduled tasks. It restarts for a moment.", "Pause"), SettingsWords.modeDialog(false))
        assertEquals("Start background work?", SettingsWords.modeDialog(true).title)
        assertEquals("Start", SettingsWords.modeDialog(true).confirm)
    }

    @Test fun hqIsBackOnceItsStartTimeChanges() {
        val w = RestartWatch("1000")
        assertFalse(w.back(true, "1000"))
        assertTrue(w.back(true, "1042"))
    }

    @Test fun hqSeenDownThenAnsweringIsBackWhateverItsStartTime() {
        val w = RestartWatch("1000")
        assertFalse(w.back(false, null))
        assertTrue(w.back(true, null))
    }

    @Test fun hqThatWasDownBeforeTheSwitchIsBackWhenItFirstAnswersWithATime() {
        assertTrue(RestartWatch(null).back(true, "2000"))
        assertFalse(RestartWatch(null).back(true, null))
    }

    @Test fun theModeAnswerNeverMovesTheSelectionByItself() {
        assertEquals(setOf("home", "safety", "keys", "messaging", "claude", "devices", "spending", "about", "background"), SettingsPage.entries.map { it.name.lowercase() }.toSet())
    }
}
