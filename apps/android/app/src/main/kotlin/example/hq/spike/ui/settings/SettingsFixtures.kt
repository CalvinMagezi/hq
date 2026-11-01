package example.hq.spike.ui.settings

import example.hq.spike.ui.firstrun.FirstRunWords
import java.time.Instant
import org.json.JSONObject

/** Fixed Settings screens for checking against the design with no hq: `--es fixture s-home`, `s-safety`, `s-keys`, `s-chat`, `s-devices`, `s-spend`, `s-about`, `s-down`. */
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
        "agent_host":{"default_host":"mac-mini","hosts":["mac-mini","native"],"sandbox_mode":"process","idle_reap_hours":6,"drive_new_watches":false,"driver_checkin_minutes":5,"driver_nudge_budget":3,"driver_no_progress_limit":4},
        "integrations":{"remote_mcp":[{"name":"diagrams","host":"example.com","live_user_turn_only":true},{"name":"search","host":"search.example.com","live_user_turn_only":false}],"searxng_host":null,"disk_watchdog_enabled":true}}""",
    )
    val chat = JSONObject(
        """{"telegram":{"token_set":true,"enabled":true,"owner":{"paired":true},"bot":{"username":"example_bot"},"relay":{"status":"connected","current":true}},"discord":{"token_set":true,"enabled":true,"relay":{"status":"connected","current":false}}}""",
    )
    val devices = JSONObject("""{"devices":[{"id":"d1","label":"MacBook","created_at":${now.minusSeconds(170_000).epochSecond},"last_used_at":${now.minusSeconds(10_800).epochSecond},"expires_at":null}]}""")
    val budgets = JSONObject("""{"budgets":[{"name":"all","scope":"global","period":"day","action":"block","limit_usd":5,"spent_usd":2.1,"pct":42,"state":"ok","resets_at":${future(32_400)}},{"name":"openrouter","scope":"provider:openrouter","period":"week","action":"notify","limit_usd":10,"spent_usd":9.5,"pct":95,"state":"warning","resets_at":${future(259_200)}}]}""")
    val forecast = JSONObject("""{"month":{"rate_per_hour":0.18,"spent_this_period":31.8,"hours_to_reset":240,"projected_period_total":74.0,"limit_usd":null,"hours_to_limit":null,"exhausts_before_reset":false,"confidence":"medium","history_hours":72,"seasonality_note":null,"windows":[]},"budgets":[],"drivers":{"models":[],"origins":[]}}""")
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
        HomeRow(SettingsPage.Background, "background", "Background work", SettingsWords.BACKGROUND_ON),
        HomeRow(SettingsPage.KeepRunning, "battery", "Keep running", "Good · 1 thing to check"),
        HomeRow(SettingsPage.Notifications, "notifications", "Notifications", "Needs you on · Updates on"),
        HomeRow(SettingsPage.CaptureWidgets, "capture", "Capture and widgets", "Sharing opens a new note"),
        HomeRow(SettingsPage.StoredKeys, "key", FirstRunWords.KEYS_TITLE, "Add, replace or remove"),
    )
    val channels = listOf(ChannelFacts("needs", "Needs you", 4, true), ChannelFacts("updates", "Updates", 3, false), ChannelFacts("hub", "HQ is running", 2, false))
    val base = SettingsUi(open = true, snap = SettingsSnapshot(settings, chat, devices, budgets, health, providers = providers, copilot = copilot, forecast = forecast), loaded = true, phone = phone, mode = "active", build = "hub \u00b7 build 1", now = now)
    return when (name) {
        "s-home" -> base
        "s-loading" -> SettingsUi(open = true, loaded = false, phone = phone, mode = "active", build = "hub \u00b7 build 1", now = now)
        "s-safety" -> base.copy(stack = listOf(SettingsPage.Safety))
        "s-time" -> base.copy(stack = listOf(SettingsPage.Safety), safetyTab = SettingsWords.SafetyTab.Time)
        "s-helpers" -> base.copy(stack = listOf(SettingsPage.Safety), safetyTab = SettingsWords.SafetyTab.Helpers)
        "s-keys" -> base.copy(stack = listOf(SettingsPage.Keys))
        "s-chat" -> base.copy(stack = listOf(SettingsPage.Messaging))
        "s-devices" -> base.copy(stack = listOf(SettingsPage.Devices))
        "s-keep" -> base.copy(stack = listOf(SettingsPage.KeepRunning), keep = KeepFacts(autoRevokeOff = true, samsung = true, lastStop = LastStop(android.app.ApplicationExitInfo.REASON_LOW_MEMORY, now.minusSeconds(172_800))))
        "s-keep-bad" -> base.copy(stack = listOf(SettingsPage.KeepRunning), keep = KeepFacts(batteryUnrestricted = false, autoRevokeOff = false, notificationsAllowed = false))
        "s-notif" -> base.copy(stack = listOf(SettingsPage.Notifications), notif = NotifFacts(true, channels, true))
        "s-notif-off" -> base.copy(stack = listOf(SettingsPage.Notifications), notif = NotifFacts(false, channels, false))
        "s-pair" -> base.copy(stack = listOf(SettingsPage.Messaging, SettingsPage.PairApp), pair = PairState("telegram", "K4M7Q-2XPA9", "/pair K4M7Q-2XPA9", "https://t.me/example_bot?start=K4M7Q-2XPA9", Instant.now().epochSecond + 272))
        "s-pair-discord" -> base.copy(stack = listOf(SettingsPage.Messaging, SettingsPage.PairApp), pair = PairState("discord", "K4M7Q-2XPA9", "!pair K4M7Q-2XPA9", null, Instant.now().epochSecond + 800), setup = SetupState("discord", appId = "123", intentOff = true))
        "s-pair-expired" -> base.copy(stack = listOf(SettingsPage.Messaging, SettingsPage.PairApp), pair = PairState("telegram", "K4M7Q-2XPA9", "/pair K4M7Q-2XPA9", null, Instant.now().epochSecond - 5))
        "s-setup" -> base.copy(stack = listOf(SettingsPage.Messaging, SettingsPage.SetupApp), setup = SetupState("telegram", found = "@example_bot"))
        "s-messaging-ask" -> base.copy(stack = listOf(SettingsPage.Messaging), askDisconnect = "telegram")
        "s-key" -> base.copy(stack = listOf(SettingsPage.Keys), keySheet = KeySheet("anthropic", "Anthropic", replace = false))
        "s-key-error" -> base.copy(stack = listOf(SettingsPage.Keys), keySheet = KeySheet("anthropic", "Anthropic", replace = false, error = SettingsWords.KEY_REFUSED))
        "s-capture" -> base.copy(stack = listOf(SettingsPage.CaptureWidgets))
        "s-capture-waiting" -> base.copy(stack = listOf(SettingsPage.CaptureWidgets), capture = CaptureFacts("task", true, listOf(
            example.hq.spike.capture.Queued("a", "note", "Why Compose beat the WebView for chat", org.json.JSONObject(), 1L),
            example.hq.spike.capture.Queued("b", "task", "Check Samsung battery whitelist", org.json.JSONObject(), 2L, "HQ did not accept it"),
        )))
        "s-background" -> base.copy(stack = listOf(SettingsPage.Background))
        "s-background-standby" -> base.copy(stack = listOf(SettingsPage.Background), mode = "standby")
        "s-background-ask" -> base.copy(stack = listOf(SettingsPage.Background), askMode = false)
        "s-background-restarting" -> base.copy(stack = listOf(SettingsPage.Background), restarting = true)
        "s-spend" -> base.copy(stack = listOf(SettingsPage.Spending))
        "s-spend-exceeded" -> base.copy(stack = listOf(SettingsPage.Spending), snap = base.snap.copy(budgets = JSONObject("""{"budgets":[{"name":"all","scope":"global","period":"day","action":"block","limit_usd":5,"spent_usd":5.2,"pct":104,"state":"exceeded","resets_at":${future(1_800)}}]}"""), forecast = JSONObject("""{"month":{"rate_per_hour":null,"spent_this_period":0,"hours_to_reset":700,"projected_period_total":null}}""")))
        "s-origins" -> base.copy(stack = listOf(SettingsPage.Safety, SettingsPage.Origins))
        "s-tools" -> base.copy(stack = listOf(SettingsPage.Safety, SettingsPage.ConnectedTools))
        "s-computers" -> base.copy(stack = listOf(SettingsPage.Safety, SettingsPage.Computers))
        "s-restart-ask" -> base.copy(stack = listOf(SettingsPage.About), askRestart = true)
        "s-about" -> base.copy(stack = listOf(SettingsPage.About))
        "s-down" -> base.copy(stack = listOf(SettingsPage.Keys), snap = SettingsSnapshot(), loaded = true)
        else -> null
    }
}
