package example.hq.spike.ui.usage

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The shapes are what /api/settings, /api/usage/providers, /api/copilot-usage and /api/budgets send, for a config whose first choice runs on GitHub Copilot. */
class UsageWordsTest {
    private fun entry(name: String, host: String, model: String, primary: Boolean = false, fallback: Int? = null) =
        """{"name":"$name","kind":"openai-compatible","model":"$model","effort":null,"endpoint_host":"$host","primary":$primary,"fallback_position":${fallback ?: "null"}}"""

    private val settings = JSONObject(
        """{"model":{"active":"gpt-6-luna","default_model":"gpt-6-luna","local_only":false},
        "provider_keys":[{"name":"OpenRouter","id":"open_router","configured":true,"source":"env"},{"name":"DeepSeek","configured":true},{"name":"Brave Search","configured":true}],
        "backends":{"configured":true,"primary":"luna","fallbacks":["copilot","haiku","deepseek","gpt-6-astra"],"entries":[
          ${entry("luna", "api.githubcopilot.com", "gpt-6-luna", primary = true)},
          ${entry("gpt-6-astra", "api.githubcopilot.com", "gpt-6-astra", fallback = 3)},
          ${entry("copilot", "api.githubcopilot.com", "claude-haiku-5.5", fallback = 0)},
          ${entry("haiku", "openrouter.ai", "anthropic/claude-haiku-5.5", fallback = 1)},
          ${entry("deepseek", "api.deepseek.com", "deepseek-flash", fallback = 2)}]}}""",
    )

    private val copilot = JSONObject(
        """{"active":true,"quota":{"login":"x","plan":"pro","entitlement":1500,"credits_used":588,"remaining":912,"percent_remaining":60.8,"overage_permitted":false,"reset_at":"2026-11-01T00:00:00Z"},
        "burn":{"windows":[{"window_hours":1,"credits_used":3,"per_hour":3},{"window_hours":6,"credits_used":50,"per_hour":8.3}],"projected_at_reset":1300,"hours_to_reset":240,"exhausts_before_reset":false,"overage_permitted":false,"sample_count":40,"confidence":"medium"}}""",
    )

    private fun providers(openrouterBalance: Double?, deepseekToday: Double = 0.42) = JSONObject(
        """{"generated_at":"2026-10-22T04:00:00Z","backends":[
          {"backend":"luna","adapter":"subscription","source":"ledger","status":"subscription","provider":null,"ledger":{"today":0.0,"week":0.0,"month":0.0,"unpriced_calls":12},"rate_limit":null,"note":"Billed through a subscription quota."},
          {"backend":"copilot","adapter":"subscription","source":"ledger","status":"subscription","provider":null,"ledger":{"today":0.0,"week":0.0,"month":0.0,"unpriced_calls":2},"rate_limit":null,"note":"Billed through a subscription quota."},
          {"backend":"gpt-6-astra","adapter":"subscription","source":"ledger","status":"subscription","provider":null,"ledger":{"today":0.0,"week":0.0,"month":0.0,"unpriced_calls":0},"rate_limit":null,"note":"Billed through a subscription quota."},
          {"backend":"haiku","adapter":"openrouter","source":"provider","status":"ok","provider":{"balance":${if (openrouterBalance == null) "null" else """{"amount":$openrouterBalance,"currency":"USD"}"""},"spend":{"today":0.31,"week":1.2,"month":4.8},"limit_remaining":null},"ledger":{"today":0.3,"week":1.1,"month":4.7,"unpriced_calls":0},"rate_limit":null,"note":null},
          {"backend":"deepseek","adapter":"deepseek","source":"ledger","status":"no_key","provider":null,"ledger":{"today":$deepseekToday,"week":1.0,"month":3.0,"unpriced_calls":0},"rate_limit":null,"note":"No key is set."}]}""",
    )

    private fun budgets(vararg b: String) = JSONObject("""{"budgets":[${b.joinToString(",")}],"definitions":[],"background_run_usd":null,"allow_unpriced_models":[],"problems":[]}""")
    private fun budget(name: String, pct: Int, state: String, scope: String = "global") =
        """{"name":"$name","scope":"$scope","period":"day","action":"notify","limit_usd":5,"spent_usd":${pct / 20.0},"pct":$pct,"state":"$state"}"""

    @Test fun theFirstChoiceIsGitHubCopilotEvenThoughAnOpenRouterKeyIsStored() {
        assertEquals("GitHub Copilot", UsageWords.primaryProvider(settings))
        assertEquals(listOf("GitHub Copilot", "OpenRouter", "DeepSeek"), UsageWords.providers(settings))
        assertEquals(listOf("GitHub Copilot", "GitHub Copilot", "OpenRouter", "DeepSeek", "GitHub Copilot"), UsageWords.chain(settings).map { it.provider })
    }

    @Test fun theChainFollowsTheOrderHqTries() {
        val chain = UsageWords.chain(settings)
        assertEquals(listOf("luna", "copilot", "haiku", "deepseek", "gpt-6-astra"), chain.map { it.backend })
        assertEquals(listOf(1, 2, 3, 4, 5), chain.map { it.position })
        assertTrue(chain.first().first)
        assertEquals("gpt-6-luna", chain.first().model)
    }

    @Test fun noOrderMeansNoChain() {
        assertTrue(UsageWords.chain(JSONObject("""{"backends":{"configured":false,"primary":"","fallbacks":[],"entries":[]}}""")).isEmpty())
        assertTrue(UsageWords.chain(null).isEmpty())
        assertNull(UsageWords.primaryProvider(JSONObject("""{"model":{}}""")))
    }

    @Test fun serviceNamesComeFromTheEndpoint() {
        assertEquals("GitHub Copilot", UsageWords.providerName("api.githubcopilot.com", "openai-compatible"))
        assertEquals("OpenRouter", UsageWords.providerName("openrouter.ai", "openai-compatible"))
        assertEquals("DeepSeek", UsageWords.providerName("api.deepseek.com", null))
        assertEquals("Anthropic", UsageWords.providerName("api.anthropic.com", null))
        assertEquals("This phone", UsageWords.providerName("127.0.0.1", "openai-compatible"))
        assertEquals("Kimi Code", UsageWords.providerName(null, "kimi-code"))
        assertEquals("example.org", UsageWords.providerName("api.example.org", null))
        assertEquals("Mybackend", UsageWords.providerName(null, null, "mybackend"))
    }

    @Test fun theChipShowsCreditsLeftOnTheServiceInUse() {
        val chip = UsageWords.chip(UsageSnapshot(settings = settings, providers = providers(4.2), copilot = copilot, budgets = budgets(budget("all", 40, "ok"))))!!
        assertEquals("Copilot 912 left", chip.label)
        assertEquals("GitHub Copilot credits: 912 of 1.5k left, 8/h over 6h", chip.title)
        assertEquals(ChipTarget.AiService, chip.target)
        assertEquals(ChipTone.Normal, chip.tone)
    }

    @Test fun aBudgetThatNeedsAttentionOutranksABalance() {
        val warn = UsageWords.chip(UsageSnapshot(settings = settings, copilot = copilot, budgets = budgets(budget("all", 10, "ok"), budget("openrouter", 85, "warning", "provider:openrouter"), budget("weekly", 120, "exceeded"))))!!
        assertEquals("weekly 120%", warn.label)
        assertEquals(ChipTone.Exceeded, warn.tone)
        assertEquals(ChipTarget.Spending, warn.target)
        val soft = UsageWords.chip(UsageSnapshot(settings = settings, copilot = copilot, budgets = budgets(budget("openrouter", 85, "warning", "provider:openrouter"))))!!
        assertEquals("openrouter 85%", soft.label)
        assertEquals(ChipTone.Warning, soft.tone)
        assertTrue(soft.title.contains("on provider:openrouter"))
    }

    @Test fun whenCopilotCannotBeReadTheNextServiceInTheChainShows() {
        val off = JSONObject("""{"active":false}""")
        assertEquals("OpenRouter $4.20 left", UsageWords.chip(UsageSnapshot(settings = settings, providers = providers(4.2), copilot = off))!!.label)
        assertEquals("OpenRouter $0.31 today", UsageWords.chip(UsageSnapshot(settings = settings, providers = providers(null), copilot = off))!!.label)
    }

    @Test fun aServiceWithNoBalanceShowsWhatItCostToday() {
        val s = JSONObject("""{"backends":{"configured":true,"primary":"deepseek","fallbacks":[],"entries":[${entry("deepseek", "api.deepseek.com", "deepseek-flash", primary = true)}]}}""")
        assertEquals("DeepSeek $0.42 today", UsageWords.chip(UsageSnapshot(settings = s, providers = providers(null)))!!.label)
    }

    @Test fun nothingKnownMeansNoChip() {
        assertNull(UsageWords.chip(UsageSnapshot()))
        assertNull(UsageWords.chip(UsageSnapshot(settings = settings, copilot = JSONObject("""{"active":true,"unavailable":true,"note":"GitHub refused the read"}"""))))
    }

    @Test fun withNoOrderSetTheWebAppsOrderApplies() {
        val none = JSONObject("""{"backends":{"configured":false}}""")
        assertEquals("Copilot 912 left", UsageWords.chip(UsageSnapshot(settings = none, providers = providers(4.2), copilot = copilot))!!.label)
    }

    @Test fun numbersAreShortAndExact() {
        assertEquals("12.9k", UsageWords.compact(12_940.0))
        assertEquals("950", UsageWords.compact(950.0))
        assertEquals("1.3M", UsageWords.compact(1_250_000.0))
        assertEquals("1.5k", UsageWords.compact(1500.0))
        assertEquals("$0.31", UsageWords.usd(0.31))
        assertEquals("$4.20", UsageWords.usd(4.2))
        assertEquals("$0.0042", UsageWords.usd(0.0042))
        assertEquals("$12.50", UsageWords.usd(12.5))
        assertEquals("-", UsageWords.usd(null))
    }

    @Test fun aPlanThatIsNotMeteredHasNoChipAndTheNextServiceShows() {
        // What the server sends for a seat with no credit allowance: a quota of zero and no burn.
        val unmetered = JSONObject("""{"active":true,"quota":{"entitlement":0,"credits_used":0,"remaining":0,"percent_remaining":0,"overage_permitted":false},"note":"This plan is not metered."}""")
        assertNull(UsageWords.copilotChip(unmetered))
        assertEquals("OpenRouter $4.20 left", UsageWords.chip(UsageSnapshot(settings = settings, providers = providers(4.2), copilot = unmetered))!!.label)
    }

    @Test fun aBackendThatIsSwitchedOffIsNotPartOfTheOrder() {
        val rowsWithoutLuna = JSONObject(providers(4.2).toString()).also { p ->
            val kept = org.json.JSONArray(); val all = p.getJSONArray("backends")
            for (i in 0 until all.length()) if (all.getJSONObject(i).getString("backend") != "luna") kept.put(all.getJSONObject(i))
            p.put("backends", kept)
        }
        // The providers route lists only backends that are on, so a backend with no row there is off.
        assertEquals("copilot", UsageWords.chain(settings, rowsWithoutLuna).first().backend)
        assertEquals(4, UsageWords.chain(settings, rowsWithoutLuna).size)
        // The settings may also say so themselves.
        val off = JSONObject(settings.toString().replace("\"name\":\"luna\",", "\"name\":\"luna\",\"enabled\":false,"))
        assertEquals("copilot", UsageWords.chain(off).first().backend)
        assertEquals(4, UsageWords.chain(off).size)
    }

    @Test fun aServiceWithNothingRecordedDoesNotHideTheNextOnesBalance() {
        val s = JSONObject("""{"backends":{"configured":true,"primary":"deepseek","fallbacks":["haiku"],"entries":[${entry("deepseek", "api.deepseek.com", "deepseek-flash", primary = true)},${entry("haiku", "openrouter.ai", "x", fallback = 0)}]}}""")
        val p = JSONObject("""{"backends":[{"backend":"deepseek","adapter":"deepseek","status":"no_key","provider":null,"ledger":{"today":0.0,"week":0.0,"month":0.0}},{"backend":"haiku","adapter":"openrouter","status":"ok","provider":{"balance":{"amount":4.2,"currency":"USD"},"spend":{"today":0.31},"limit_remaining":null},"ledger":{"today":0.3}}]}""")
        assertEquals("OpenRouter $4.20 left", UsageWords.chip(UsageSnapshot(settings = s, providers = p))!!.label)
    }

    @Test fun aBalanceInAnotherCurrencyKeepsItsCode() {
        val s = JSONObject("""{"backends":{"configured":true,"primary":"deepseek","fallbacks":[],"entries":[${entry("deepseek", "api.deepseek.com", "deepseek-flash", primary = true)}]}}""")
        val p = JSONObject("""{"backends":[{"backend":"deepseek","adapter":"deepseek","status":"ok","provider":{"balance":{"amount":50.0,"currency":"CNY"},"spend":null,"limit_remaining":null},"ledger":{"today":0.0}}]}""")
        assertEquals("DeepSeek 50.00 CNY left", UsageWords.chip(UsageSnapshot(settings = s, providers = p))!!.label)
        assertEquals("$4.20", UsageWords.money(4.2, "USD"))
        assertEquals("$4.20", UsageWords.money(4.2, null))
    }

    @Test fun aGatewayIsNamedByItsHostAndALocalEndpointWithAPortIsThisPhone() {
        assertEquals("Kimi Code", UsageWords.providerName("api.kimi.com", "anthropic-compatible"))
        assertEquals("Anthropic", UsageWords.providerName("api.anthropic.com", "anthropic-compatible"))
        assertEquals("Anthropic", UsageWords.providerName(null, "anthropic-compatible"))
        assertEquals("This phone", UsageWords.providerName("localhost:11434", "openai-compatible"))
        assertEquals("This phone", UsageWords.providerName("127.0.0.1:8080", "openai-compatible"))
        assertEquals("This phone", UsageWords.providerName("[::1]:8080", "openai-compatible"))
    }

    @Test fun aFailedUsageReadIsNotSaidAsIfTheAiWereDown() {
        assertEquals("Balance not readable", UsageWords.statusWords("refused"))
        assertEquals("Usage not readable", UsageWords.statusWords("error"))
    }

    @Test fun aStatusIsSaidInPlainWords() {
        assertEquals("Subscription", UsageWords.statusWords("subscription"))
        assertEquals("No key set", UsageWords.statusWords("no_key"))
        assertEquals("", UsageWords.statusWords("something-new"))
    }

    @Test fun aNegativeAmountKeepsItsSignBeforeTheDollar() {
        org.junit.Assert.assertEquals("-$0.04", UsageWords.usd(-0.04))
        org.junit.Assert.assertEquals("-$12.50", UsageWords.usd(-12.5))
        org.junit.Assert.assertEquals("-$0.0001", UsageWords.usd(-0.0001))
        org.junit.Assert.assertEquals("$2.973", UsageWords.usd(2.973))
    }
}
