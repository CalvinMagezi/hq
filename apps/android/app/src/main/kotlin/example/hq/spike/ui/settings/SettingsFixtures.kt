package example.hq.spike.ui.settings

import java.time.Instant
import org.json.JSONObject

/** Fixed Settings screens for checking against the design with no hq: `--es fixture s-home`, `s-safety`, `s-keys`, `s-chat`, `s-claude`, `s-devices`, `s-spend`, `s-about`, `s-down`. */
fun settingsFixture(name: String?): SettingsUi? {
    val now = Instant.now()
    fun future(seconds: Long) = now.plusSeconds(seconds).epochSecond
    val settings = JSONObject(
        """{"model":{"active":"gpt-6-luna","default_model":"gpt-6-luna","local_only":false},
        "provider_keys":[{"name":"OpenRouter","id":"open_router","configured":true,"source":"env"},{"name":"Anthropic","id":"anthropic","configured":false,"source":null},{"name":"Google AI","id":"google","configured":false,"source":null},{"name":"DeepSeek","configured":true},{"name":"OpenAI","configured":false},{"name":"Brave Search","configured":true}],
        "backends":{"configured":true,"primary":"luna","fallbacks":["copilot","haiku","deepseek"],"entries":[
          {"name":"luna","kind":"openai-compatible","model":"gpt-6-luna","endpoint_host":"api.githubcopilot.com","primary":true,"fallback_position":null},
          {"name":"copilot","kind":"openai-compatible","model":"claude-haiku-5.5","endpoint_host":"api.githubcopilot.com","primary":false,"fallback_position":0},
          {"name":"haiku","kind":"openai-compatible","model":"anthropic/claude-haiku-5.5","endpoint_host":"openrouter.ai","primary":false,"fallback_position":1},
          {"name":"deepseek","kind":"openai-compatible","model":"deepseek-flash","endpoint_host":"api.deepseek.com","primary":false,"fallback_position":2}]},
        "limits":{"chat_turn_timeout_secs":600,"turn_ack_timeout_secs":270,"background_turn_max_days":5,"background_progress_secs":300},
        "safety":{"bash_sandbox":"required","bash_network":false,"skills_write_approval":false,"web_allowed_origins":["https://hq.example.ts.net"]},
        "agent_host":{"default_host":"mac-mini","hosts":["mac-mini"],"sandbox_mode":"process","idle_reap_hours":6,"drive_new_watches":false,"driver_checkin_minutes":5,"driver_nudge_budget":3,"driver_no_progress_limit":4},
        "integrations":{"remote_mcp":[],"searxng_host":null,"disk_watchdog_enabled":true}}""",
    )
    val chat = JSONObject(
        """{"telegram":{"token_set":true,"enabled":true,"owner":{"paired":true},"bot":{"username":"example_bot"},"relay":{"status":"connected","current":true}},"discord":{"token_set":true,"enabled":true,"relay":{"status":"connected","current":false}}}""",
    )
    val connector = JSONObject(
        """{"enabled":true,"url":"https://hq.example.ts.net/mcp","armed_until":${future(581)},"grants":[{"id":"g1","label":"Claude","grant_scope":"read_only","created_at":${now.minusSeconds(90_000).epochSecond},"expires_at":${future(86_400)},"last_used_at":null,"revoked_at":null}]}""",
    )
    val devices = JSONObject("""{"devices":[{"id":"d1","label":"MacBook","created_at":${now.minusSeconds(170_000).epochSecond},"last_used_at":${now.minusSeconds(10_800).epochSecond},"expires_at":null}]}""")
    val budgets = JSONObject("""{"budgets":[{"name":"all","scope":"global","period":"day","limit_usd":5,"spent_usd":2.1,"state":"ok"},{"name":"openrouter","scope":"provider:openrouter","period":"week","limit_usd":10,"spent_usd":9.5,"state":"warn"}]}""")
    val health = JSONObject("""{"status":"ok","service":"hq","version":"0.9.1"}""")
    val providers = JSONObject(
        """{"generated_at":"2026-10-22T04:00:00Z","backends":[
        {"backend":"luna","adapter":"subscription","source":"ledger","status":"subscription","provider":null,"ledger":{"today":0.0,"week":0.0,"month":0.0,"unpriced_calls":12},"rate_limit":null,"note":null},
        {"backend":"copilot","adapter":"subscription","source":"ledger","status":"subscription","provider":null,"ledger":{"today":0.0,"week":0.0,"month":0.0,"unpriced_calls":3},"rate_limit":null,"note":null},
        {"backend":"haiku","adapter":"openrouter","source":"provider","status":"ok","provider":{"balance":{"amount":4.2,"currency":"USD"},"spend":{"today":0.31,"week":1.2,"month":4.8},"limit_remaining":null},"ledger":{"today":0.3,"week":1.1,"month":4.7,"unpriced_calls":0},"rate_limit":null,"note":null},
        {"backend":"deepseek","adapter":"deepseek","source":"ledger","status":"no_key","provider":null,"ledger":{"today":0.42,"week":1.0,"month":3.0,"unpriced_calls":0},"rate_limit":null,"note":"No key is set for this backend."}]}""",
    )
    val copilot = JSONObject(
        """{"active":true,"quota":{"login":"example","plan":"pro","entitlement":1500,"credits_used":588,"remaining":912,"percent_remaining":60.8,"overage_permitted":false,"reset_at":"2026-11-01T00:00:00Z"},"burn":{"windows":[{"window_hours":6,"credits_used":50,"per_hour":8.3}]}}""",
    )
    val phone = listOf(
        HomeRow(null, "background", "Background work", "On · answers chats, runs tasks"),
        HomeRow(null, "battery", "Keep running", "Good · 1 thing to check"),
        HomeRow(null, "notifications", "Notifications", "Needs you on · Updates on"),
        HomeRow(null, "capture", "Capture and widgets", "Sharing opens a new note"),
    )
    val base = SettingsUi(open = true, snap = SettingsSnapshot(settings, chat, connector, devices, budgets, health, providers = providers, copilot = copilot), loaded = true, phone = phone, mode = "active", build = "hub \u00b7 build 1", now = now)
    return when (name) {
        "s-home" -> base
        "s-loading" -> SettingsUi(open = true, loaded = false, phone = phone, mode = "active", build = "hub \u00b7 build 1", now = now)
        "s-safety" -> base.copy(stack = listOf(SettingsPage.Safety))
        "s-time" -> base.copy(stack = listOf(SettingsPage.Safety), safetyTab = SettingsWords.SafetyTab.Time)
        "s-helpers" -> base.copy(stack = listOf(SettingsPage.Safety), safetyTab = SettingsWords.SafetyTab.Helpers)
        "s-keys" -> base.copy(stack = listOf(SettingsPage.Keys))
        "s-chat" -> base.copy(stack = listOf(SettingsPage.Messaging))
        "s-claude" -> base.copy(stack = listOf(SettingsPage.Claude))
        "s-devices" -> base.copy(stack = listOf(SettingsPage.Devices))
        "s-spend" -> base.copy(stack = listOf(SettingsPage.Spending))
        "s-about" -> base.copy(stack = listOf(SettingsPage.About))
        "s-down" -> base.copy(stack = listOf(SettingsPage.Keys), snap = SettingsSnapshot(), loaded = true)
        else -> null
    }
}
