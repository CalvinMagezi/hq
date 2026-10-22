package example.hq.spike.ui.settings

import java.time.Instant
import java.time.ZoneOffset
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SettingsWordsTest {
    private val now = Instant.parse("2026-10-21T12:00:00Z")

    @org.junit.Before fun english() = java.util.Locale.setDefault(java.util.Locale.ENGLISH)

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

    /** /api/settings for a config whose first choice runs on GitHub Copilot while an OpenRouter and a DeepSeek key are also stored. */
    private val onCopilot = JSONObject(
        """{"model":{"active":"gpt-6-luna","default_model":"gpt-6-luna","local_only":false},
        "provider_keys":[{"name":"OpenRouter","id":"open_router","configured":true,"source":"env"},{"name":"DeepSeek","configured":true},{"name":"Brave Search","configured":true}],
        "backends":{"configured":true,"primary":"luna","fallbacks":["copilot","haiku","deepseek"],"entries":[
          {"name":"luna","kind":"openai-compatible","model":"gpt-6-luna","endpoint_host":"api.githubcopilot.com","primary":true,"fallback_position":null},
          {"name":"copilot","kind":"openai-compatible","model":"claude-haiku-5.5","endpoint_host":"api.githubcopilot.com","primary":false,"fallback_position":0},
          {"name":"haiku","kind":"openai-compatible","model":"anthropic/claude-haiku-5.5","endpoint_host":"openrouter.ai","primary":false,"fallback_position":1},
          {"name":"deepseek","kind":"openai-compatible","model":"deepseek-flash","endpoint_host":"api.deepseek.com","primary":false,"fallback_position":2}]}}""",
    )
    private val copilotUsage = JSONObject("""{"active":true,"quota":{"entitlement":1500,"remaining":912,"reset_at":"2026-11-01T00:00:00Z"},"burn":{"windows":[]}}""")
    private val providerRows = JSONObject(
        """{"backends":[
        {"backend":"luna","adapter":"subscription","source":"ledger","status":"subscription","provider":null,"ledger":{"today":0.0,"week":0.0,"month":0.0}},
        {"backend":"copilot","adapter":"subscription","source":"ledger","status":"subscription","provider":null,"ledger":{"today":0.0,"week":0.0,"month":0.0}},
        {"backend":"haiku","adapter":"openrouter","source":"provider","status":"ok","provider":{"balance":{"amount":4.2,"currency":"USD"},"spend":{"today":0.31,"week":1.2,"month":4.8},"limit_remaining":null},"ledger":{"today":0.3,"week":1.1,"month":4.7}},
        {"backend":"deepseek","adapter":"deepseek","source":"ledger","status":"no_key","provider":null,"ledger":{"today":0.42,"week":1.0,"month":3.0}}]}""",
    )
    private val snapOnCopilot = SettingsSnapshot(settings = onCopilot, providers = providerRows, copilot = copilotUsage)

    @Test fun theAiServiceLineNamesTheOneHqUsesNotTheFirstStoredKey() {
        assertEquals("GitHub Copilot · 3 services", SettingsWords.keysStatus(onCopilot))
    }

    @Test fun theKeysPageListsCopilotAndTheOrderHqTries() {
        assertEquals("GitHub Copilot", SettingsWords.keyRows(onCopilot, copilotUsage).first().first)
        assertTrue(SettingsWords.keyRows(onCopilot, copilotUsage).first().second)
        assertEquals(false, SettingsWords.keyRows(onCopilot, JSONObject("""{"active":false}""")).first().second)
        val rows = SettingsWords.aiRows(snapOnCopilot)
        assertEquals(listOf("1 · GitHub Copilot", "2 · GitHub Copilot", "3 · OpenRouter", "4 · DeepSeek"), rows.map { it.title })
        assertEquals("First choice", rows[0].value)
        assertEquals("If the one before is down", rows[1].value)
        assertEquals("gpt-6-luna · Subscription", rows[0].text)
        assertEquals("deepseek-flash · No key set", rows[3].text)
    }

    @Test fun spendingIsShownPerServiceWithWhatIsLeft() {
        val rows = SettingsWords.serviceRows(snapOnCopilot)
        assertEquals(listOf("GitHub Copilot", "OpenRouter", "DeepSeek"), rows.map { it.title })
        assertEquals("912 of 1.5k credits left", rows[0].value)
        assertEquals("Credits on your plan · Resets 1 Nov", rows[0].text)
        assertEquals("$4.20 left", rows[1].value)
        assertEquals("Today $0.31 · This week $1.20 · This month $4.80", rows[1].text)
        assertEquals("$0.42 today", rows[2].value)
        assertEquals("Today $0.42 · This week $1.00 · This month $3.00 (HQ's own record)", rows[2].text)
    }

    @Test fun aCopilotTokenWhoseUsageCannotBeReadIsNotShownAsNotSet() {
        val unreadable = JSONObject("""{"active":true,"unavailable":true,"note":"GitHub refused the usage read"}""")
        val row = SettingsWords.keyRows(onCopilot, unreadable).first()
        assertEquals(Triple("GitHub Copilot", false, "unreadable"), row)
        assertEquals(Triple("GitHub Copilot", false, "unreadable"), SettingsWords.keyRows(onCopilot, null).first())
    }

    @Test fun aPlanWithNoCreditAllowanceSaysItIsNotMetered() {
        val unmetered = JSONObject("""{"active":true,"quota":{"entitlement":0,"remaining":0},"note":"This plan is not metered."}""")
        assertEquals("Not metered", SettingsWords.serviceRows(snapOnCopilot.copy(copilot = unmetered)).first().value)
        assertEquals(Triple("GitHub Copilot", true, "env"), SettingsWords.keyRows(onCopilot, unmetered).first())
    }

    @Test fun aServiceReportingChinaYuanIsNotShownInDollars() {
        val cny = JSONObject("""{"backends":[{"backend":"deepseek","adapter":"deepseek","status":"ok","provider":{"balance":{"amount":50.0,"currency":"CNY"},"spend":null,"limit_remaining":null},"ledger":{"today":0.0}}]}""")
        val s = JSONObject("""{"backends":{"configured":true,"primary":"deepseek","fallbacks":[],"entries":[{"name":"deepseek","kind":"openai-compatible","model":"m","endpoint_host":"api.deepseek.com","primary":true,"fallback_position":null}]}}""")
        assertEquals("50.00 CNY left", SettingsWords.serviceRows(SettingsSnapshot(settings = s, providers = cny)).first().value)
    }

    @Test fun aCopilotPlanThatCannotBeReadSaysSoInsteadOfGuessing() {
        val unreadable = snapOnCopilot.copy(copilot = JSONObject("""{"active":true,"unavailable":true,"note":"GitHub refused the usage read"}"""))
        assertEquals("Not readable", SettingsWords.serviceRows(unreadable).first().value)
        assertEquals("Not measured", SettingsWords.serviceRows(snapOnCopilot.copy(copilot = null)).first().value)
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
        val b = JSONObject("""{"budgets":[{"name":"global","scope":"global","period":"week","limit_usd":20,"spent_usd":4.5},{"name":"daily","scope":"global","period":"day","limit_usd":5,"spent_usd":2.1}]}""")
        assertEquals("$2.10 of $5.00 today", SettingsWords.spendingStatus(b))
        assertEquals("No limit set", SettingsWords.spendingStatus(JSONObject("""{"budgets":[]}""")))
    }

    @Test fun aBudgetForOneProviderIsNotPresentedAsTodaysTotal() {
        val b = JSONObject("""{"budgets":[{"name":"openrouter","scope":"provider:openrouter","period":"day","limit_usd":1,"spent_usd":0.4},{"name":"all","scope":"global","period":"month","limit_usd":50,"spent_usd":12}]}""")
        assertEquals("$12.00 of $50.00 this month", SettingsWords.spendingStatus(b))
        val only = JSONObject("""{"budgets":[{"name":"openrouter","scope":"provider:openrouter","period":"day","limit_usd":1,"spent_usd":0.4}]}""")
        assertEquals("1 limits set", SettingsWords.spendingStatus(only))
    }

    private val at = java.time.Instant.parse("2026-10-22T12:00:00Z")

    @Test fun eachBudgetCardSaysWhatItCoversHowMuchIsUsedAndWhenItResets() {
        val b = JSONObject("""{"budgets":[{"name":"daily cap","scope":"global","period":"day","action":"block","limit_usd":5,"spent_usd":2.1,"pct":42,"state":"ok","resets_at":${at.epochSecond + 9 * 3600}},{"name":"openrouter","scope":"provider:openrouter","period":"week","action":"notify","limit_usd":10,"spent_usd":9.5,"pct":95,"state":"warning","resets_at":${at.epochSecond + 3 * 86_400}}]}""")
        val cards = SettingsWords.budgetCards(b, at)
        assertEquals("Daily cap", cards[0].title)
        assertEquals("global, daily, then block", cards[0].covers)
        assertEquals("$2.10 of $5.00 (42%)", cards[0].used)
        assertEquals("Within budget, resets in 9h", cards[0].stand)
        assertEquals(0.42f, cards[0].fraction, 0.001f)
        assertEquals("provider:openrouter, weekly, then notify", cards[1].covers)
        assertEquals("Past the alert threshold, resets in 3 days", cards[1].stand)
        assertEquals(SettingsWords.BudgetTone.Warning, cards[1].tone)
    }

    @Test fun aBudgetPastItsLimitFillsTheBarAndSaysSo() {
        val b = JSONObject("""{"budgets":[{"name":"all","scope":"global","period":"day","limit_usd":5,"spent_usd":5.2,"pct":104,"state":"exceeded","resets_at":${at.epochSecond + 1800}}]}""")
        val c = SettingsWords.budgetCards(b, at).single()
        assertEquals("$5.20 of $5.00 (104%)", c.used)
        assertEquals(1f, c.fraction, 0f)
        assertEquals("Limit reached, resets in <1h", c.stand)
        assertEquals(SettingsWords.BudgetTone.Exceeded, c.tone)
    }

    @Test fun aBudgetWithoutAPercentOrResetStillReads() {
        val c = SettingsWords.budgetCards(JSONObject("""{"budgets":[{"name":"x","scope":"global","period":"month","limit_usd":50,"spent_usd":12}]}"""), at).single()
        assertEquals("$12.00 of $50.00 (24%)", c.used)
        assertEquals("Within budget", c.stand)
        assertEquals("global, monthly", c.covers)
    }

    @Test fun spansReadAsTheBoardWritesThem() {
        assertEquals("<1h", SettingsWords.span(0.4))
        assertEquals("<1h", SettingsWords.span(-3.0))
        assertEquals("9h", SettingsWords.span(9.2))
        assertEquals("1 day", SettingsWords.span(24.0))
        assertEquals("10 days", SettingsWords.span(240.0))
    }

    @Test fun theForecastGivesThreeRowsOrSaysThereIsNothingToForecast() {
        val f = SettingsWords.forecast(JSONObject("""{"month":{"rate_per_hour":0.18,"spent_this_period":31.8,"hours_to_reset":240,"projected_period_total":74.0}}"""))!!
        assertEquals(listOf("Spent so far" to "$31.80", "Projected month end" to "$74.00", "Resets in" to "10 days"), f.rows.map { it.title to it.value })
        assertNull(f.note)
        val none = SettingsWords.forecast(JSONObject("""{"month":{"rate_per_hour":null,"spent_this_period":0,"hours_to_reset":700,"projected_period_total":null}}"""))!!
        assertEquals("No spend recorded yet, so there is nothing to forecast.", none.note)
        assertEquals(0, none.rows.size)
        assertNull(SettingsWords.forecast(JSONObject("""{}""")))
        assertNull(SettingsWords.forecast(null))
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
        // The server sends only process or none for the helpers' sandbox, and required, best_effort or off for the commands one.
        fun helpers(mode: String) = JSONObject("""{"agent_host":{"sandbox_mode":"$mode","hosts":[]}}""").let { SettingsWords.safetyRows(it, SettingsWords.SafetyTab.Helpers)[2].value }
        assertEquals("Fenced in, internet limited to a list", helpers("process"))
        assertEquals("Off", helpers("none"))
        fun commands(mode: String) = JSONObject("""{"safety":{"bash_sandbox":"$mode","web_allowed_origins":[]}}""").let { SettingsWords.safetyRows(it, SettingsWords.SafetyTab.Safety)[0].value }
        assertEquals("When the phone can", commands("best_effort"))
        assertEquals("Required", commands("required"))
        assertEquals("Off", commands("off"))
    }

    @Test fun theHelpersTabNamesThePhoneAndSaysWhatTheNoProgressLimitReallyDoes() {
        val s = JSONObject("""{"agent_host":{"default_host":"native","hosts":[],"sandbox_mode":"process","driver_no_progress_limit":3}}""")
        val rows = SettingsWords.safetyRows(s, SettingsWords.SafetyTab.Helpers)
        assertEquals("This phone", rows[0].value)
        val limit = rows.last()
        assertEquals("3 turns", limit.value)
        assertEquals("HQ stops answering its questions for you. The helper itself keeps running.", limit.text)
    }

    @Test fun aSearchKeyIsNotCountedAsAnAiKey() {
        val s = JSONObject("""{"provider_keys":[{"name":"OpenRouter","configured":true},{"name":"DeepSeek","configured":true},{"name":"Brave Search","configured":true}]}""")
        assertEquals("OpenRouter · 2 keys", SettingsWords.keysStatus(s))
        val onlySearch = JSONObject("""{"provider_keys":[{"name":"Brave Search","configured":true}]}""")
        assertEquals("No key yet", SettingsWords.keysStatus(onlySearch))
    }

    @Test fun aRelayFileLeftByAnEarlierRunIsNotARunningRelay() {
        val chat = JSONObject("""{"telegram":{"token_set":true,"enabled":true,"owner":{"paired":true},"relay":{"status":"connected","current":false}},"discord":{"token_set":true,"enabled":true,"relay":{"status":"connected","current":true}}}""")
        assertEquals("Not running yet", SettingsWords.appCard(chat, "telegram", "Telegram").line)
        assertEquals("Running", SettingsWords.appCard(chat, "discord", "Discord").line)
    }

    @Test fun noOrderedListOfAisIsTheFlatDefaultNotNoAi() {
        val s = JSONObject("""{"backends":{"configured":false,"primary":"","fallbacks":[],"entries":[]}}""")
        assertEquals(listOf("Whichever key is set" to "no order is set"), SettingsWords.fallbackRows(s))
    }

    @Test fun theHomeListSaysItIsCheckingUntilItHasAskedAndNeverThenSaysNotAnswering() {
        val groups = SettingsWords.home(SettingsSnapshot(), now, listOf(HomeRow(null, "background", "Background work", "On")), loaded = false)
        assertEquals("On", groups[0].rows.single().status)
        assertTrue(groups.drop(1).flatMap { it.rows }.all { it.status == "Checking…" })
        assertTrue(SettingsWords.home(SettingsSnapshot(), now, emptyList(), loaded = true).drop(1).flatMap { it.rows }.any { it.status == "Not answering" })
    }

    @Test fun anOffsetInIsoTextIsReadWithoutInstantParse() {
        val o = JSONObject("""{"a":"2026-10-21T12:00:00+00:00","b":"2026-10-21T14:00:00+02:00","c":"2026-10-21 12:00:00"}""")
        val noon = Instant.parse("2026-10-21T12:00:00Z")
        assertEquals(noon, SettingsWords.time(o, "a"))
        assertEquals(noon, SettingsWords.time(o, "b"))
        assertEquals(noon, SettingsWords.time(o, "c"))
    }

    @Test fun numbersDoNotTakeThePhonesLocale() {
        val before = java.util.Locale.getDefault()
        try {
            java.util.Locale.setDefault(java.util.Locale.GERMANY)
            assertEquals("9:05", SettingsWords.clock(545))
            assertEquals("$2.10 of $5.00 today", SettingsWords.spendingStatus(JSONObject("""{"budgets":[{"scope":"global","period":"day","limit_usd":5,"spent_usd":2.1}]}""")))
        } finally {
            java.util.Locale.setDefault(before)
        }
    }

    @Test fun aForecastWithNoRateButSomeSpendStillShowsTheMonthSoFar() {
        val f = SettingsWords.forecast(JSONObject("""{"month":{"rate_per_hour":null,"spent_this_period":3.4,"hours_to_reset":500,"projected_period_total":null,"confidence":"low"}}"""))!!
        assertNull(f.note)
        assertEquals(listOf("$3.40", "-", "21 days"), f.rows.map { it.value })
        assertEquals("Not much history yet, so this is a rough guess.", f.rows[1].text)
    }

    @Test fun theEmptyBudgetsLineDoesNotClaimNothingIsLimitedWhenBackgroundRunsAreCapped() {
        assertEquals("No budgets are set, so nothing is limited.", SettingsWords.noBudgets(JSONObject("""{"budgets":[]}""")))
        assertEquals("No budgets are set.", SettingsWords.noBudgets(JSONObject("""{"budgets":[],"background_run_usd":2.5}""")))
    }

    @Test fun theNotesCountIsShownOnlyWhenThisHqSaysIt() {
        fun snap(json: String?) = SettingsSnapshot(vault = json?.let { org.json.JSONObject(it) })
        org.junit.Assert.assertEquals("2,860", SettingsWords.notesCount(snap("""{"vault_path":"/x","notes":2860}""")))
        org.junit.Assert.assertEquals("0", SettingsWords.notesCount(snap("""{"notes":0}""")))
        org.junit.Assert.assertNull(SettingsWords.notesCount(snap("""{"vault_path":"/x"}""")))
        org.junit.Assert.assertNull(SettingsWords.notesCount(snap("""{"notes":null}""")))
        org.junit.Assert.assertNull(SettingsWords.notesCount(snap(null)))
    }
}
