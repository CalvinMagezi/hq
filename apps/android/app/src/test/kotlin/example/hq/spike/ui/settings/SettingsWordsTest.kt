package example.hq.spike.ui.settings

import java.time.Instant
import java.time.ZoneOffset
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class SettingsWordsTest {
    private val now = Instant.parse("2026-10-21T12:00:00Z")

    private val settings = JSONObject(
        """{"model":{"active":"openrouter/auto","default_model":"openrouter/auto"},
        "provider_keys":[{"name":"OpenRouter","id":"open_router","configured":true,"source":"config"},{"name":"Anthropic","id":"anthropic","configured":false,"source":null},{"name":"Groq","configured":false}],
        "backends":{"configured":true,"primary":"openrouter","fallbacks":["luna"]},
        "limits":{"chat_turn_timeout_secs":600,"turn_ack_timeout_secs":270,"background_turn_max_days":5,"background_progress_secs":300},
        "safety":{"bash_sandbox":"required","bash_network":false,"skills_write_approval":false,"web_allowed_origins":["https://hq.example.ts.net"]},
        "agent_host":{"default_host":"mac-mini","hosts":["mac-mini","laptop"],"sandbox_mode":"best_effort","idle_reap_hours":6,"drive_new_watches":false,"driver_checkin_minutes":5,"driver_nudge_budget":3,"driver_no_progress_limit":4},
        "integrations":{"remote_mcp":[],"searxng_host":null,"disk_watchdog_enabled":true}}""",
    )

    @Test fun theKeysLineNamesTheFirstAndCountsThem() {
        assertEquals("OpenRouter · 1 key", SettingsWords.keysStatus(settings))
        assertEquals("No key yet", SettingsWords.keysStatus(JSONObject("""{"provider_keys":[{"name":"OpenRouter","configured":false}]}""")))
        assertEquals("Not answering", SettingsWords.keysStatus(null))
    }

    @Test fun messagingNeedsBothTheTokenAndTheSwitch() {
        val chat = JSONObject("""{"telegram":{"token_set":true,"enabled":true},"discord":{"token_set":true,"enabled":false}}""")
        assertEquals("Telegram on · Discord off", SettingsWords.messagingStatus(chat))
        assertEquals("Not answering", SettingsWords.messagingStatus(null))
    }

    @Test fun theConnectorSaysHowLongItStaysOpenAndHowManyAreIn() {
        val open = JSONObject(
            """{"enabled":true,"armed_until":"2026-10-21T12:09:41Z","grants":[
            {"id":"g1","label":"Claude","grant_scope":"read_only","created_at":"2026-10-20T10:00:00Z","expires_at":"2026-11-20T10:00:00Z"},
            {"id":"g2","grant_scope":"full","created_at":"2026-10-01T10:00:00Z","revoked_at":"2026-10-02T10:00:00Z"},
            {"id":"g3","grant_scope":"full","created_at":"2026-09-01T10:00:00Z","expires_at":"2026-10-01T10:00:00Z"}]}""",
        )
        assertEquals("Open for 9:41 · 1 connected", SettingsWords.connectorStatus(open, now))
        assertEquals("Closed · none connected", SettingsWords.connectorStatus(JSONObject("""{"enabled":true,"grants":[]}"""), now))
        assertEquals("Off", SettingsWords.connectorStatus(JSONObject("""{"enabled":false}"""), now))
        assertEquals("Read only", SettingsWords.grants(open, now).single().scope)
    }

    @Test fun anExpiredWindowIsClosed() {
        val c = JSONObject("""{"enabled":true,"armed_until":"2026-10-21T11:00:00Z","grants":[]}""")
        assertEquals("Closed · none connected", SettingsWords.connectorStatus(c, now))
    }

    @Test fun devicesAreCounted() {
        assertEquals("1 signed in", SettingsWords.devicesStatus(JSONObject("""{"devices":[{"id":"d1","label":"MacBook","created_at":"2026-10-20 10:00:00"}]}""")))
        assertEquals("None signed in", SettingsWords.devicesStatus(JSONObject("""{"devices":[]}""")))
    }

    @Test fun spendingShowsTodaysBudgetInDollars() {
        val b = JSONObject("""{"budgets":[{"name":"global","period":"week","limit_usd":20,"spent_usd":4.5},{"name":"daily","period":"day","limit_usd":5,"spent_usd":2.1}]}""")
        assertEquals("$2.10 of $5.00 today", SettingsWords.spendingStatus(b))
        assertEquals("No limit set", SettingsWords.spendingStatus(JSONObject("""{"budgets":[]}""")))
    }

    @Test fun theSafetyTabsReadTheServersNumbers() {
        val safety = SettingsWords.safetyRows(settings, SettingsWords.SafetyTab.Safety)
        assertEquals(listOf("Required", "No", "No", "1"), safety.map { it.value })
        val time = SettingsWords.safetyRows(settings, SettingsWords.SafetyTab.Time)
        assertEquals(listOf("10 min", "270 s", "5 days", "5 min"), time.map { it.value })
        val helpers = SettingsWords.safetyRows(settings, SettingsWords.SafetyTab.Helpers)
        assertEquals("mac-mini", helpers.first().value)
        assertEquals("When the phone can", helpers[2].value)
        assertEquals("6 h", helpers[3].value)
        val tools = SettingsWords.safetyRows(settings, SettingsWords.SafetyTab.Tools)
        assertEquals(listOf("0", "Built in", "On"), tools.map { it.value })
    }

    @Test fun noAnswerMeansNoRows() {
        assertEquals(emptyList<FactRow>(), SettingsWords.safetyRows(null, SettingsWords.SafetyTab.Safety))
    }

    @Test fun theModelAndFallbackOrderAreRead() {
        assertEquals("openrouter/auto", SettingsWords.activeModel(settings))
        assertEquals(listOf("1 · openrouter" to "primary", "2 · luna" to "fallback"), SettingsWords.fallbackRows(settings))
        assertEquals(listOf("1 · None set" to "no AI is set", "2 · None set" to "no fallback"), SettingsWords.fallbackRows(JSONObject("""{"backends":{"fallbacks":[]}}""")))
    }

    @Test fun aMessagingCardSaysRunningOffOrRestart() {
        val chat = JSONObject(
            """{"telegram":{"token_set":true,"enabled":true,"needs_restart":false,"owner":{"paired":true},"bot":{"username":"example_bot"},"relay":{"status":"connected"}},
            "discord":{"token_set":false,"enabled":false}}""",
        )
        val t = SettingsWords.appCard(chat, "telegram", "Telegram")
        assertEquals("Running", t.line)
        assertEquals("@example_bot", t.bot)
        assertEquals("Linked", t.owner)
        assertEquals("Off", SettingsWords.appCard(chat, "discord", "Discord").line)
        val saved = JSONObject("""{"telegram":{"token_set":true,"enabled":true,"needs_restart":true}}""")
        assertEquals("Saved · restart needed", SettingsWords.appCard(saved, "telegram", "Telegram").line)
    }

    @Test fun homeHasTheBoardsGroupsInOrder() {
        val groups = SettingsWords.home(SettingsSnapshot(settings = settings), now)
        assertEquals(listOf("This phone", "Your accounts", "How HQ behaves", "Your data"), groups.map { it.title })
        assertEquals(listOf("AI service and keys", "Messaging apps", "Connect Claude", "Other devices"), groups[1].rows.map { it.title })
        assertEquals("Read only", groups[2].rows.single().status)
        assertNull(groups[0].rows.firstOrNull())
    }

    // The server sends every connector and device time as epoch seconds.
    private val nowSec = now.epochSecond

    private val realConnector = JSONObject(
        """{"enabled":true,"url":"https://hq-connector.example.com/mcp","consent_origin":"https://hq.example.com","default_scope":"full","armed_until":${nowSec + 581},
        "grants":[{"id":"g1","client_id":"c1","client_name":"Claude","label":null,"grant_scope":"full","created_at":${nowSec - 90_000},"expires_at":${nowSec + 86_400},"last_used_at":null,"revoked_at":null,"revoked_reason":null},
        {"id":"g2","client_name":"Old","grant_scope":"read_only","created_at":${nowSec - 900_000},"expires_at":${nowSec - 60},"revoked_at":null},
        {"id":"g3","client_name":"Gone","grant_scope":"full","created_at":${nowSec - 900_000},"expires_at":${nowSec + 500},"revoked_at":${nowSec - 5}}]}""",
    )

    @Test fun epochSecondsAreReadForTheConnectorWindowAndTheGrants() {
        assertEquals("Open for 9:41 · 1 connected", SettingsWords.connectorStatus(realConnector, now))
        val grants = SettingsWords.grants(realConnector, now, ZoneOffset.UTC)
        assertEquals(1, grants.size)
        assertEquals("Claude", grants.single().client)
        assertEquals("Full", grants.single().scope)
        assertEquals("Oct 20", grants.single().since)
    }

    @Test fun aWindowInThePastIsClosedAndAnExpiredOrRevokedGrantIsNotCounted() {
        val c = JSONObject(realConnector.toString().replace("\"armed_until\":${nowSec + 581}", "\"armed_until\":${nowSec - 30}"))
        assertEquals("Closed · 1 connected", SettingsWords.connectorStatus(c, now))
        assertEquals(listOf("g1"), SettingsWords.liveGrants(c, now).map { it.optString("id") })
        assertEquals("Closed · none connected", SettingsWords.connectorStatus(JSONObject("""{"enabled":true,"armed_until":null,"grants":[]}"""), now))
    }

    @Test fun deviceTimesAreDatesAndHowLongAgoNeverRawNumbers() {
        val d = JSONObject("""{"devices":[{"id":"d1","label":"Signed in with a code","created_at":${nowSec - 170_000},"last_used_at":${nowSec - 10_800},"expires_at":null},{"id":"d2","label":"MacBook","created_at":${nowSec - 90_000},"last_used_at":null,"expires_at":null}]}""")
        val list = SettingsWords.devices(d, now, ZoneOffset.UTC)
        assertEquals("Signed in Oct 19 · used 3 h ago", list[0].line)
        assertEquals("Signed in Oct 20", list[1].line)
        assertEquals("2 signed in", SettingsWords.devicesStatus(d))
    }

    @Test fun aTimeIsReadFromSecondsMillisecondsDigitsAndText() {
        val o = JSONObject("""{"a":1792591376,"b":1792591376000,"c":"1792591376","d":"2026-10-21 12:00:00","e":"2026-10-21T12:00:00+00:00","f":null,"g":"soon"}""")
        val expected = Instant.ofEpochSecond(1792591376)
        assertEquals(expected, SettingsWords.time(o, "a"))
        assertEquals(expected, SettingsWords.time(o, "b"))
        assertEquals(expected, SettingsWords.time(o, "c"))
        assertEquals(Instant.parse("2026-10-21T12:00:00Z"), SettingsWords.time(o, "d"))
        assertEquals(Instant.parse("2026-10-21T12:00:00Z"), SettingsWords.time(o, "e"))
        assertNull(SettingsWords.time(o, "f"))
        assertNull(SettingsWords.time(o, "g"))
        assertNull(SettingsWords.time(o, "missing"))
        assertNull(SettingsWords.time(null, "a"))
    }

    @Test fun theSandboxIsNamedInWordsNotConfigValues() {
        fun row(mode: String) = JSONObject("""{"agent_host":{"sandbox_mode":"$mode","hosts":[]}}""").let { SettingsWords.safetyRows(it, SettingsWords.SafetyTab.Helpers)[2].value }
        assertEquals("In a separate process", row("process"))
        assertEquals("Off", row("none"))
        assertEquals("When the phone can", row("best_effort"))
        assertEquals("Required", row("required"))
    }
}
