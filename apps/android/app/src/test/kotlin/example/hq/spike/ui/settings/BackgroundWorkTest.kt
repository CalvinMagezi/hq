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

    private val settings = JSONObject("""{"safety":{"web_allowed_origins":["https://hq.example.ts.net","http://localhost:5678"]},"agent_host":{"default_host":"mac-mini","hosts":["mac-mini","native"]},"integrations":{"remote_mcp":[{"name":"diagrams","host":"example.com","live_user_turn_only":true},{"name":"search","host":"search.example.com","live_user_turn_only":false}]}}""")

    @Test fun theSafetyCountsOpenTheirListsWhenTheyHaveRows() {
        val safety = SettingsWords.safetyRows(settings, SettingsWords.SafetyTab.Safety)
        assertEquals(SettingsPage.Origins, safety.first { it.title.startsWith("Addresses") }.opens)
        assertEquals(SettingsPage.Computers, SettingsWords.safetyRows(settings, SettingsWords.SafetyTab.Helpers).first { it.title == "Computers" }.opens)
        assertEquals(SettingsPage.ConnectedTools, SettingsWords.safetyRows(settings, SettingsWords.SafetyTab.Tools).first { it.title == "Connected tools" }.opens)
        val none = JSONObject("""{"safety":{"web_allowed_origins":[]},"integrations":{"remote_mcp":[]},"agent_host":{"hosts":[]}}""")
        assertEquals(null, SettingsWords.safetyRows(none, SettingsWords.SafetyTab.Tools).first { it.title == "Connected tools" }.opens)
    }

    @Test fun theListsReadTheirRows() {
        assertEquals(listOf("https://hq.example.ts.net", "http://localhost:5678"), SettingsWords.originRows(settings))
        val tools = SettingsWords.toolRows(settings)
        assertEquals("diagrams", tools[0].title)
        assertEquals("example.com\nUsed only while you are in the chat.", tools[0].text)
        assertEquals("search.example.com", tools[1].text)
        assertEquals(listOf("mac-mini" to "Default", "This phone" to null), SettingsWords.computerRows(settings).map { it.title to it.text })
        assertEquals(emptyList<String>(), SettingsWords.originRows(null))
    }

    @Test fun diagnosticsSayWhatRunsAndWhereItServes() {
        val on = SettingsWords.diagnostics("hub \u00b7 build 1", true, JSONObject("""{"status":"ok"}"""), 5678)
        assertEquals("AgentHQ hub build hub \u00b7 build 1\nRun mode: Active: relays and scheduler on. Serving this device on 127.0.0.1:5678\nHealth: {\"status\":\"ok\"}\n", on)
        assertEquals("AgentHQ hub build x\nRun mode: Standby: web app only. Serving this device on 127.0.0.1:5678\nHealth: no answer\n", SettingsWords.diagnostics("x", false, null, 5678))
    }

    @Test fun theHomeRowAdmitsThereIsNoBackup() {
        val row = SettingsWords.home(SettingsSnapshot(), java.time.Instant.EPOCH).flatMap { it.rows }.first { it.page == SettingsPage.About }
        assertEquals("Backup and about", row.title)
        assertEquals("No backup yet", row.status)
        assertTrue(row.warn)
    }
}
