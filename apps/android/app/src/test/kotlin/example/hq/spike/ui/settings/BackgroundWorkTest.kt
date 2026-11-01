package example.hq.spike.ui.settings

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
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

    @Test fun onlyTheThreeCheckableProvidersGetAKeyButton() {
        val s = JSONObject("""{"provider_keys":[{"name":"OpenRouter","id":"open_router","configured":true,"source":"env"},{"name":"Anthropic","id":"anthropic","configured":false,"source":null},{"name":"Google AI","id":"google","configured":true,"source":"file"},{"name":"DeepSeek","configured":true}]}""")
        assertNull(SettingsWords.keyAction(s, "OpenRouter", true, "env"))
        assertEquals(SettingsWords.KeyAction("anthropic", "Add"), SettingsWords.keyAction(s, "Anthropic", false, null))
        assertEquals(SettingsWords.KeyAction("google", "Replace"), SettingsWords.keyAction(s, "Google AI", true, "file"))
        assertNull(SettingsWords.keyAction(s, "DeepSeek", true, null))
    }

    @Test fun theKeySheetSaysAddOrReplaceAndWhyItFailed() {
        assertEquals("Add your Anthropic key", SettingsWords.keyWords("Anthropic", false).title)
        assertEquals("Replace your Google AI key", SettingsWords.keyWords("Google AI", true).title)
        assertEquals(SettingsWords.KEY_REFUSED, SettingsWords.keyFailure(400))
        assertEquals(SettingsWords.KEY_NO_ANSWER, SettingsWords.keyFailure(0))
        assertEquals(SettingsWords.KEY_ENV, SettingsWords.keyFailure(409))
        assertEquals(SettingsWords.KEY_NO_ANSWER, SettingsWords.keyFailure(503))
        assertEquals(SettingsWords.KEY_OTHER, SettingsWords.keyFailure(403))
        assertEquals("Anthropic key saved.", SettingsWords.keySaved("Anthropic"))
        assertEquals("https://openrouter.ai/keys", SettingsWords.keyHelpUrl("open_router"))
        assertNull(SettingsWords.keyHelpUrl("deepseek"))
    }

    @Test fun aSignInCodeIsReadInTwoGroupsOfFive() {
        assertEquals("8KQ3M-W2T7P", SettingsWords.groupCode("8KQ3MW2T7P"))
        assertEquals("SHORT", SettingsWords.groupCode("SHORT"))
        assertEquals("Works once, for 9:41. Enter the code on the other device.", SettingsWords.codeHelp("9:41"))
    }

    @Test fun theSignOutEverywhereDialogSaysClaudeEndsToo() {
        assertEquals(true, SettingsWords.SIGN_OUT_ALL_WORDS.body.contains("Claude connections end too"))
        assertEquals("Sign out", SettingsWords.SIGN_OUT_ALL_WORDS.confirm)
    }

    @Test fun aRelaySaysWhenItWasLastSeen() {
        val now = java.time.Instant.parse("2026-10-22T14:00:00Z")
        val chat = JSONObject("""{"telegram":{"enabled":true,"token_set":true,"relay":{"status":"connected","current":true,"last_seen":${now.epochSecond - 12}}},"discord":{"enabled":false,"token_set":false}}""")
        assertEquals("Running \u00b7 seen 12 s ago", SettingsWords.appCard(chat, "telegram", "Telegram", now).line)
        assertEquals("12 s ago", SettingsWords.seen(now.minusSeconds(12), now))
        assertEquals("5 min ago", SettingsWords.seen(now.minusSeconds(300), now))
        assertEquals("2 h ago", SettingsWords.seen(now.minusSeconds(7200), now))
    }

    @Test fun setupWordsFollowTheWebAppsSteps() {
        assertEquals(4, SettingsWords.setupSteps("telegram").size)
        assertEquals("Open @BotFather in Telegram.", SettingsWords.setupSteps("telegram").first())
        assertEquals("Paste the token below.", SettingsWords.setupSteps("discord").last())
        assertEquals("https://t.me/BotFather", SettingsWords.setupUrl("telegram"))
        assertEquals("https://discord.com/oauth2/authorize?client_id=42&scope=bot%20applications.commands&permissions=309237730368", SettingsWords.inviteUrl("42"))
    }

    @Test fun pairingTellsWhereToSendTheCodeWithOrWithoutAButton() {
        assertEquals("Tap the button to open your bot and press Start. Or send it /pair\u00A0K4M7Q-2XPA9 yourself.", SettingsWords.pairHelp("telegram", "/pair K4M7Q-2XPA9", true))
        assertEquals("Open your bot in Telegram and send it /pair\u00A0K4M7Q-2XPA9.", SettingsWords.pairHelp("telegram", "/pair K4M7Q-2XPA9", false))
        assertEquals("Open a direct message with your bot in Discord and send !pair\u00A0K4M7Q-2XPA9.", SettingsWords.pairHelp("discord", "!pair K4M7Q-2XPA9", false))
        assertEquals("Waiting for the message. The code works for 4:32.", SettingsWords.pairWaiting("4:32"))
    }

    @Test fun aChatAppFailureUsesTheServersWordsWhenItGaveSome() {
        assertEquals("HQ did not reply. Try again.", SettingsWords.chatAppFailure(0, null))
        assertEquals("Token rejected by Telegram", SettingsWords.chatAppFailure(400, "Token rejected by Telegram"))
        assertEquals(SettingsWords.TOKEN_REFUSED, SettingsWords.chatAppFailure(400, null))
        assertEquals("Disconnect Telegram?", SettingsWords.disconnectWords("telegram").title)
    }
}
