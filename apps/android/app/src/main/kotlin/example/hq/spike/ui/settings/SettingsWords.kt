package example.hq.spike.ui.settings

import java.time.Duration
import java.time.Instant
import java.time.format.DateTimeParseException
import example.hq.spike.ui.usage.ChainRow
import example.hq.spike.ui.usage.UsageWords
import org.json.JSONArray
import org.json.JSONObject

/** What each Settings route answered; a null is a route that did not answer, so its rows say so instead of guessing. */
data class SettingsSnapshot(
    val settings: JSONObject? = null,
    val chatApps: JSONObject? = null,
    val connector: JSONObject? = null,
    val devices: JSONObject? = null,
    val budgets: JSONObject? = null,
    val health: JSONObject? = null,
    /** Routes HQ answered with an error (not found, a server error): the page says that, not that HQ is not running. */
    val unavailable: Set<String> = emptySet(),
    /** What each AI service in use says about its balance and spend (/api/usage/providers), and the GitHub Copilot credits (/api/copilot-usage). */
    val providers: JSONObject? = null,
    val copilot: JSONObject? = null,
    /** This month's spend and where it is heading (/api/usage/forecast). */
    val forecast: JSONObject? = null,
    /** What HQ says about the vault (/api/vault-status): its path and, from a build that has it, how many notes are indexed. */
    val vault: JSONObject? = null,
)

enum class SettingsPage { Home, Safety, Keys, Messaging, Claude, Devices, Spending, About, Background, KeepRunning, Notifications, Origins, ConnectedTools, Computers, CaptureWidgets, SetupApp, PairApp, StoredKeys }

data class HomeRow(val page: SettingsPage?, val key: String, val title: String, val status: String, val warn: Boolean = false)

data class HomeGroup(val title: String, val rows: List<HomeRow>)

/** A line on a read-only page: a title with its value, and a sentence that says what it means. */
data class FactRow(val title: String, val value: String, val text: String? = null, val good: Boolean = false, val opens: SettingsPage? = null)

/** The words of the Settings board, built from what the server sent. Pure, so each line can be tested from a real response. */
object SettingsWords {
    private const val NOT_ANSWERED = "Not answering"
    private const val PERCENT = 100.0
    private const val SECONDS_PER_HOUR = 3_600.0
    private const val HOURS_PER_DAY = 24.0
    private val RESET_DAY: java.time.format.DateTimeFormatter get() = java.time.format.DateTimeFormatter.ofPattern("d MMM", java.util.Locale.getDefault())

    fun yesNo(v: Boolean) = if (v) "Yes" else "No"
    fun onOff(v: Boolean) = if (v) "On" else "Off"

    private fun JSONObject?.obj(name: String): JSONObject? = this?.optJSONObject(name)
    private fun JSONObject?.arr(name: String): JSONArray? = this?.optJSONArray(name)
    private fun JSONArray?.items(): List<JSONObject> = if (this == null) emptyList() else (0 until length()).mapNotNull { optJSONObject(it) }
    private fun JSONObject?.text(name: String): String? = this?.takeIf { has(name) && !isNull(name) }?.optString(name)?.takeIf { it.isNotBlank() }

    /** Keys for search or other tools, not for an AI service: they are listed but not counted as "keys" on the home line. */
    private val TOOL_KEYS = setOf("Brave Search")

    fun configuredKeys(s: JSONObject?): List<String> = s.arr("provider_keys").items().filter { it.optBoolean("configured") && it.optString("name") !in TOOL_KEYS }.map { it.optString("name") }

    fun keysStatus(s: JSONObject?, providers: JSONObject? = null): String {
        if (s == null) return NOT_ANSWERED
        // The AI HQ uses is the first one in its chain, whichever keys are stored: a stored key is not a choice.
        val services = UsageWords.providers(s, providers)
        if (services.isNotEmpty()) return "${services.first()} · ${services.size} ${if (services.size == 1) "service" else "services"}"
        val keys = configuredKeys(s)
        if (keys.isEmpty()) return "No key yet"
        return "${keys.first()} · ${keys.size} ${if (keys.size == 1) "key" else "keys"}"
    }

    private fun app(chat: JSONObject?, name: String): String = if (chat.obj(name)?.let { it.optBoolean("enabled") && it.optBoolean("token_set") } == true) "on" else "off"

    fun messagingStatus(chat: JSONObject?): String = if (chat == null) NOT_ANSWERED else "Telegram ${app(chat, "telegram")} · Discord ${app(chat, "discord")}"

    private const val MILLIS_FROM = 100_000_000_000L

    /** A time from a JSON field: epoch seconds (what the server sends), epoch milliseconds, or ISO text. Null when absent or unreadable. */
    fun time(o: JSONObject?, name: String): Instant? {
        val v = o?.takeIf { it.has(name) && !it.isNull(name) }?.opt(name) ?: return null
        return when (v) {
            is Number -> epoch(v.toLong())
            is String -> v.trim().let { t -> t.toLongOrNull()?.let(::epoch) ?: iso(t) }
            else -> null
        }
    }

    /** ISO text with an offset (+00:00, Z) or none (taken as UTC). OffsetDateTime reads the offsets that Instant.parse refuses before Android 14. */
    private fun iso(t: String): Instant? {
        val text = t.replace(' ', 'T')
        return try { java.time.OffsetDateTime.parse(text).toInstant() } catch (e: DateTimeParseException) {
            try { java.time.LocalDateTime.parse(text).toInstant(java.time.ZoneOffset.UTC) } catch (e2: DateTimeParseException) { null }
        }
    }

    private fun epoch(n: Long): Instant = if (n >= MILLIS_FROM) Instant.ofEpochMilli(n) else Instant.ofEpochSecond(n)

    private fun dayOf(t: Instant?, zone: java.time.ZoneId): String = t?.atZone(zone)?.format(java.time.format.DateTimeFormatter.ofPattern("MMM d", java.util.Locale.getDefault())).orEmpty()

    /** "3 h ago": how long before [now], for a "used" line. */
    fun ago(t: Instant?, now: Instant): String {
        if (t == null) return ""
        val minutes = Duration.between(t, now).toMinutes().coerceAtLeast(0)
        return when {
            minutes < 1 -> "just now"
            minutes < 60 -> "$minutes min ago"
            minutes < 24 * 60 -> "${minutes / 60} h ago"
            else -> "${minutes / (24 * 60)} d ago"
        }
    }

    /** Grants that still let a client in: not revoked and not past their end. */
    fun liveGrants(c: JSONObject?, now: Instant): List<JSONObject> =
        c.arr("grants").items().filter { g -> time(g, "revoked_at") == null && (time(g, "expires_at")?.isAfter(now) ?: true) }

    fun clock(seconds: Long): String = String.format(java.util.Locale.ROOT, "%d:%02d", seconds / 60, seconds % 60)

    fun connectorStatus(c: JSONObject?, now: Instant): String {
        if (c == null) return NOT_ANSWERED
        if (!c.optBoolean("enabled")) return "Off"
        val left = time(c, "armed_until")?.let { Duration.between(now, it).seconds }?.takeIf { it > 0 }
        val n = liveGrants(c, now).size
        val connected = if (n == 0) "none connected" else "$n connected"
        return (if (left != null) "Open for ${clock(left)}" else "Closed") + " · $connected"
    }

    fun devicesStatus(d: JSONObject?): String {
        if (d == null) return NOT_ANSWERED
        val n = d.arr("devices").items().size
        return if (n == 0) "None signed in" else "$n signed in"
    }

    /** "2,860" for the About page, or null when this HQ does not say how many notes there are (an older build). */
    fun notesCount(snap: SettingsSnapshot): String? {
        val v = snap.vault ?: return null
        if (!v.has("notes") || v.isNull("notes")) return null
        return v.optLong("notes", -1L).takeIf { it >= 0 }?.let { String.format(java.util.Locale.ROOT, "%,d", it) }
    }

    private fun money(v: Double) = "$" + String.format(java.util.Locale.ROOT, "%.2f", v)

    fun spendingStatus(b: JSONObject?): String {
        if (b == null) return NOT_ANSWERED
        val all = b.arr("budgets").items()
        // The whole-spend budget (scope global), daily if there is one; a budget for one provider or model is not today's total.
        val head = all.firstOrNull { it.optString("scope") == "global" && it.optString("period") == "day" } ?: all.firstOrNull { it.optString("scope") == "global" }
        if (head == null) return if (all.isEmpty()) "No limit set" else "${all.size} limits set"
        val period = when (head.optString("period")) { "day" -> "today"; "week" -> "this week"; "month" -> "this month"; else -> "" }
        return "${money(head.optDouble("spent_usd"))} of ${money(head.optDouble("limit_usd"))} $period".trim()
    }

    fun home(snap: SettingsSnapshot, now: Instant = Instant.now(), phone: List<HomeRow> = emptyList(), loaded: Boolean = true): List<HomeGroup> = withChecking(loaded, listOf(
        HomeGroup("This phone", phone),
        HomeGroup(
            "Your accounts", listOf(
                HomeRow(SettingsPage.Keys, "key", "AI service and keys", keysStatus(snap.settings, snap.providers)),
                HomeRow(SettingsPage.Messaging, "chat", "Messaging apps", messagingStatus(snap.chatApps)),
                HomeRow(SettingsPage.Claude, "plug", "Connect Claude", connectorStatus(snap.connector, now)),
                HomeRow(SettingsPage.Devices, "phone", "Other devices", devicesStatus(snap.devices)),
            ),
        ),
        HomeGroup("How HQ behaves", listOf(HomeRow(SettingsPage.Safety, "shield", "Safety and limits", if (snap.settings == null) NOT_ANSWERED else "Read only"))),
        HomeGroup(
            "Your data", listOf(
                HomeRow(SettingsPage.Spending, "gauge", "Spending limits", spendingStatus(snap.budgets)),
                HomeRow(SettingsPage.About, "arch", "Backup and about", NO_BACKUP, warn = true),
            ),
        ),
    ))

    /** Until the first read has finished, the rows that come from HQ say so, rather than "Not answering". */
    private fun withChecking(loaded: Boolean, groups: List<HomeGroup>): List<HomeGroup> =
        if (loaded) groups else groups.map { g -> if (g.title == "This phone") g else g.copy(rows = g.rows.map { it.copy(status = "Checking…") }) }

    /** No route says whether a backup was made, so this stays until one does; it is never inferred. */
    const val NO_BACKUP = "No backup yet"

    fun aboutStatus(h: JSONObject?): String = h?.text("version")?.let { "HQ $it" } ?: if (h == null) NOT_ANSWERED else "HQ is running"

    // ------------------------------------------------------------ Safety and limits (read only, four tabs)

    enum class SafetyTab(val label: String) { Safety("Safety"), Time("Time limits"), Helpers("Helpers"), Tools("Tools") }

    private fun secs(v: Int): String = when {
        v <= 0 -> "Off"
        v % 3600 == 0 -> "${v / 3600} h"
        v % 60 == 0 -> "${v / 60} min"
        else -> "$v s"
    }

    private fun sandbox(v: String): String = when (v) {
        "required" -> "Required"
        "best_effort" -> "When the phone can"
        "process" -> "Fenced in, internet limited to a list"
        "off", "none", "" -> "Off"
        else -> v.replaceFirstChar(Char::uppercase)
    }

    /** The computer a helper runs on: the phone itself is "native" in the config. */
    private fun computerName(host: String) = if (host == "native" || host == "local") "This phone" else host

    /** A count that opens its list, unless there is nothing in it to open. */
    private fun countRow(title: String, n: Int, page: SettingsPage, text: String? = null) = FactRow(title, "$n", text, opens = if (n > 0) page else null)

    /** The web addresses allowed to open HQ, one per row. */
    fun originRows(s: JSONObject?): List<String> = s.obj("safety").arr("web_allowed_origins").let { a -> if (a == null) emptyList() else (0 until a.length()).map { a.optString(it) }.filter { it.isNotBlank() } }

    /** Each tool server HQ can call: its name, where it lives, and when it is used. */
    fun toolRows(s: JSONObject?): List<FactRow> = s.obj("integrations").arr("remote_mcp").items().map {
        val lines = listOfNotNull(it.text("host"), "Used only while you are in the chat.".takeIf { _ -> it.optBoolean("live_user_turn_only") })
        FactRow(it.text("name") ?: "Tool", "", lines.joinToString("\n").ifBlank { null })
    }

    /** The computers helpers can run on; the default one says so. */
    fun computerRows(s: JSONObject?): List<FactRow> {
        val host = s.obj("agent_host")
        val default = host.text("default_host")
        val a = host.arr("hosts") ?: return emptyList()
        return (0 until a.length()).mapNotNull { i -> a.optString(i).takeIf { it.isNotBlank() } }.map { FactRow(computerName(it), "", if (it == default) "Default" else null) }
    }

    fun safetyRows(s: JSONObject?, tab: SafetyTab): List<FactRow> {
        if (s == null) return emptyList()
        val safety = s.obj("safety")
        val limits = s.obj("limits")
        val host = s.obj("agent_host")
        val integ = s.obj("integrations")
        return when (tab) {
            SafetyTab.Safety -> listOf(
                FactRow("Run commands in a safe box", sandbox(safety.text("bash_sandbox").orEmpty()), "Fences in the commands a helper runs, when the phone can."),
                FactRow("Commands can use the internet", yesNo(safety?.optBoolean("bash_network", true) ?: true), "Lets those commands download things and reach websites."),
                FactRow("Ask me before HQ changes its skills", yesNo(safety?.optBoolean("skills_write_approval") == true), "When on, a change to one of HQ's skills waits for your yes."),
                countRow("Addresses allowed to open HQ", safety.arr("web_allowed_origins")?.length() ?: 0, SettingsPage.Origins, "The web addresses that may show HQ's screens."),
            )
            SafetyTab.Time -> listOf(
                FactRow("Longest time for one reply", secs(limits?.optInt("chat_turn_timeout_secs") ?: 0)),
                FactRow("How long a reply can take before it moves to the background", secs(limits?.optInt("turn_ack_timeout_secs") ?: 0)),
                FactRow("How long background work can run", "${limits?.optInt("background_turn_max_days") ?: 0} days"),
                FactRow("How often long work checks in", secs(limits?.optInt("background_progress_secs") ?: 0)),
            )
            SafetyTab.Helpers -> listOf(
                FactRow("Default computer", host.text("default_host")?.let(::computerName) ?: "None set"),
                countRow("Computers", host.arr("hosts")?.length() ?: 0, SettingsPage.Computers),
                FactRow("Safe box for helpers", sandbox(host.text("sandbox_mode").orEmpty())),
                FactRow("Stop idle helpers after", host?.optInt("idle_reap_hours")?.let { if (it <= 0) "Never" else "$it h" } ?: "Never"),
                FactRow("New helpers: HQ answers their questions", yesNo(host?.optBoolean("drive_new_watches") == true)),
                FactRow("How often HQ checks on helpers", host?.optInt("driver_checkin_minutes")?.let { "$it min" } ?: "Not set"),
                FactRow("How many instructions HQ may give one helper", "${host?.optInt("driver_nudge_budget") ?: 0}"),
                FactRow("Stop answering a helper that makes no progress after", "${host?.optInt("driver_no_progress_limit") ?: 0} turns", "HQ stops answering its questions for you. The helper itself keeps running."),
            )
            SafetyTab.Tools -> listOf(
                countRow("Connected tools", integ.arr("remote_mcp")?.length() ?: 0, SettingsPage.ConnectedTools),
                FactRow("Search server", integ.text("searxng_host") ?: "Built in"),
                FactRow("Disk space check", onOff(integ?.optBoolean("disk_watchdog_enabled") == true)),
            )
        }
    }

    // ------------------------------------------------------------ Add a key

    /** The button a key row gets: only the three providers HQ can check have one, and a key set by the environment cannot be changed here. */
    data class KeyAction(val id: String, val label: String)

    fun keyAction(s: JSONObject?, name: String, set: Boolean, source: String?): KeyAction? {
        if (source == "env") return null
        val id = s.arr("provider_keys").items().firstOrNull { it.optString("name") == name }?.text("id") ?: return null
        return KeyAction(id, if (set) "Replace" else "Add")
    }

    data class KeyWords(val title: String, val body: String)

    fun keyWords(name: String, replace: Boolean) = KeyWords(
        if (replace) "Replace your $name key" else "Add your $name key",
        "A key is a long code that lets HQ use an AI service. Copy it from your $name account, then paste it here.",
    )

    const val KEY_REFUSED = "That key didn't work. Check that you copied all of it."
    const val KEY_NO_ANSWER = "HQ did not reply. Try again."
    const val KEY_ENV = "This key is set outside the app. Change it where it was set."
    const val KEY_OTHER = "HQ could not save the key. Try again later."
    const val KEY_NOTE = "HQ keeps the key on this phone and never shows it again."

    /** What the sheet says when a key was not saved: the key was refused, the environment owns it, or HQ did not answer (not blamed on the key). */
    fun keyFailure(status: Int): String = when {
        status == 0 || status >= HTTP_SERVER -> KEY_NO_ANSWER
        status == HTTP_CONFLICT -> KEY_ENV
        status == HTTP_BAD_KEY -> KEY_REFUSED
        else -> KEY_OTHER
    }

    private const val HTTP_SERVER = 500
    private const val HTTP_CONFLICT = 409
    private const val HTTP_BAD_KEY = 400

    fun keySaved(name: String) = "$name key saved."

    /** Where to find a provider's key, as the web app points. */
    fun keyHelpUrl(id: String): String? = when (id) {
        "open_router" -> "https://openrouter.ai/keys"
        "anthropic" -> "https://console.anthropic.com/settings/keys"
        "google" -> "https://aistudio.google.com/apikey"
        else -> null
    }

    // ------------------------------------------------------------ AI service and keys

    /** The stored API keys, with the GitHub Copilot plan first when HQ uses it: its token is set outside the key list, so the plan answering is what says it works. */
    fun keyRows(s: JSONObject?, copilot: JSONObject? = null, providers: JSONObject? = null): List<Triple<String, Boolean, String?>> {
        val stored = s.arr("provider_keys").items().map { Triple(it.optString("name"), it.optBoolean("configured"), it.text("source")) }
        val usesCopilot = UsageWords.providers(s, providers).contains(UsageWords.COPILOT)
        // The plan answering is what says the token works. When it cannot be read, that is said, not "Not set".
        val known = copilot.obj("quota") != null
        return if (usesCopilot) listOf(Triple(UsageWords.COPILOT, known, if (known) "env" else "unreadable")) + stored else stored
    }

    /** The AI services in the order HQ tries them, each with its model and whether the provider says it works. */
    fun aiRows(snap: SettingsSnapshot): List<FactRow> = UsageWords.chain(snap.settings, snap.providers).map { r ->
        val status = UsageWords.statusWords(UsageWords.providerRow(snap.providers, r.backend)?.optString("status"))
        FactRow("${r.position} · ${r.provider}", if (r.first) "First choice" else "If the one before is down", listOf(r.model, status).filter { it.isNotBlank() }.joinToString(" · ").ifBlank { null })
    }

    // ------------------------------------------------------------ Spending by AI service

    /** What each AI service in use has left and what it cost: credits for a plan, a balance where the provider gives one, else HQ's own record. */
    fun serviceRows(snap: SettingsSnapshot): List<FactRow> {
        val chain = UsageWords.chain(snap.settings, snap.providers)
        val services: List<ChainRow> = if (chain.isNotEmpty()) chain.distinctBy { it.provider } else snap.providers.arr("backends").items().map {
            val name = it.optString("backend")
            ChainRow(name, UsageWords.providerName(null, null, name), "", 0)
        }
        return services.map { r -> if (r.provider == UsageWords.COPILOT) copilotRow(snap.copilot) else providerRow(r, UsageWords.providerRow(snap.providers, r.backend)) }
    }

    private fun copilotRow(c: JSONObject?): FactRow {
        val q = c.obj("quota")
        val left = q?.takeIf { c?.optBoolean("active") == true }?.optDouble("remaining", Double.NaN)?.takeIf { it.isFinite() }
        val total = q?.optDouble("entitlement", Double.NaN)?.takeIf { it.isFinite() }
        val value = when {
            left != null && c?.has("burn") == true -> "${UsageWords.compact(left)}${total?.let { " of ${UsageWords.compact(it)}" }.orEmpty()} credits left"
            q != null && c?.optBoolean("active") == true -> "Not metered"
            c?.optBoolean("unavailable") == true || c?.has("error") == true -> "Not readable"
            else -> "Not measured"
        }
        val reset = q.text("reset_at")?.let { iso(it)?.let { t -> "Resets ${RESET_DAY.format(t.atZone(java.time.ZoneId.systemDefault()))}" } }
        return FactRow(UsageWords.COPILOT, value, listOfNotNull("Credits on your plan", reset).joinToString(" · "))
    }

    private fun providerRow(r: ChainRow, row: JSONObject?): FactRow {
        val reading = row.obj("provider")
        val ledger = row.obj("ledger")
        fun num(o: JSONObject?, name: String): Double? = o?.takeIf { it.has(name) && !it.isNull(name) }?.optDouble(name)?.takeIf { it.isFinite() }
        val left = UsageWords.leftText(reading)
        val spend = reading.obj("spend") ?: ledger
        val today = UsageWords.todayAmount(row)
        val value = when {
            left != null -> "$left left"
            today != null -> "${UsageWords.usd(today)} today"
            else -> UsageWords.statusWords(row?.optString("status")).ifBlank { "Not measured" }
        }
        val windows = listOfNotNull(today?.let { "Today ${UsageWords.usd(it)}" }, num(spend, "week")?.let { "This week ${UsageWords.usd(it)}" }, num(spend, "month")?.let { "This month ${UsageWords.usd(it)}" })
        val fromLedger = reading == null && ledger != null
        return FactRow(r.provider, value, (windows.joinToString(" · ") + if (fromLedger && windows.isNotEmpty()) " (HQ's own record)" else "").ifBlank { null })
    }

    fun activeModel(s: JSONObject?): String = s.obj("model").text("active") ?: s.obj("model").text("default_model") ?: "Not set"

    fun fallbackRows(s: JSONObject?): List<Pair<String, String>> {
        val b = s.obj("backends") ?: return emptyList()
        if (!b.optBoolean("configured", true)) return listOf("Whichever key is set" to "no order is set")
        val primary = b.text("primary")
        val fallbacks = b.arr("fallbacks")?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty()
        val first = primary?.let { listOf("1 · $it" to "primary") } ?: listOf("1 · None set" to "no AI is set")
        return first + if (fallbacks.isEmpty()) listOf("2 · None set" to "no fallback") else fallbacks.mapIndexed { i, f -> "${i + 2} · $f" to "fallback" }
    }

    // ------------------------------------------------------------ Messaging apps

    data class AppCard(val name: String, val on: Boolean, val line: String, val bot: String?, val owner: String)

    fun appCard(chat: JSONObject?, key: String, name: String, now: Instant = Instant.now()): AppCard {
        val a = chat.obj(key)
        val on = a?.let { it.optBoolean("enabled") && it.optBoolean("token_set") } == true
        val relay = a.obj("relay")
        val status = relay.text("status")
        // `current` is false when the status file was written before this HQ started (an earlier Active run), so nothing is answering now.
        val current = relay?.optBoolean("current", true) != false
        val line = when {
            !on -> "Off"
            !current -> "Not running yet"
            status == "connected" || status == "running" || status == "online" -> "Running" + (time(relay, "last_seen")?.let { " \u00b7 seen ${seen(it, now)}" }.orEmpty())
            status != null -> status.replaceFirstChar(Char::uppercase)
            else -> "Saved"
        } + if (a?.optBoolean("needs_restart") == true) " · restart needed" else ""
        val bot = a.obj("bot").text("username")?.let { if (key == "telegram") "@$it" else it }
        val paired = a.obj("owner")?.optBoolean("paired") == true
        return AppCard(name, on, line, bot, if (paired) "Linked" else "Not linked")
    }

    // ------------------------------------------------------------ Background work

    const val BACKGROUND_ON = "On \u00b7 answers chats, runs tasks"
    const val BACKGROUND_OFF = "Only when I open the app"

    /** The three lines under "Right now": hq itself, and each chat app as far as the run mode lets it be connected. */
    fun rightNow(snap: SettingsSnapshot, active: Boolean, restarting: Boolean): List<FactRow> {
        val hq = when {
            restarting -> FactRow("HQ", "Restarting")
            snap.health != null -> FactRow("HQ", "Running", good = true)
            else -> FactRow("HQ", NOT_ANSWERED)
        }
        return listOf(hq, chatAppRow(snap.chatApps, "telegram", "Telegram", active), chatAppRow(snap.chatApps, "discord", "Discord", active))
    }

    private fun chatAppRow(chat: JSONObject?, key: String, name: String, active: Boolean): FactRow {
        val a = chat.obj(key)
        val relay = a.obj("relay")
        val status = relay.text("status")
        val value = when {
            a == null || !(a.optBoolean("enabled") && a.optBoolean("token_set")) -> "Not set up"
            !active -> "Paused"
            relay?.optBoolean("current", true) == false -> "Not connected yet"
            status == "connected" -> "Connected"
            status == "error" -> "Error"
            else -> "Not connected yet"
        }
        return FactRow(name, value)
    }

    data class ModeWords(val title: String, val body: String, val confirm: String)

    val RESTART_WORDS = ModeWords("Restart HQ?", "HQ stops for a moment and starts again. Anything it is answering right now is cut off.", "Restart")

    /** The text "Copy diagnostics" puts on the clipboard: the build, what runs, where it serves, and what HQ says about itself. */
    fun diagnostics(build: String, active: Boolean, health: JSONObject?, port: Int): String = buildString {
        appendLine("AgentHQ hub build $build")
        appendLine("Run mode: ${if (active) "Active: relays and scheduler on" else "Standby: web app only"}. Serving this device on 127.0.0.1:$port")
        appendLine("Health: ${health?.toString() ?: "no answer"}")
    }

    /** What the dialog says before the run mode changes: what stops or starts, and that hq restarts for a moment. */
    fun modeDialog(toActive: Boolean) = if (toActive) {
        ModeWords("Start background work?", "HQ will start answering your chats and running scheduled tasks, even when the app is closed. It restarts for a moment.", "Start")
    } else {
        ModeWords("Pause background work?", "HQ will stop answering your chats and running scheduled tasks. It restarts for a moment.", "Pause")
    }

    /** "12 s ago", "3 min ago", "2 h ago": how long since a relay last said it was there. */
    fun seen(t: Instant, now: Instant): String {
        val s = Duration.between(t, now).seconds.coerceAtLeast(0)
        return when {
            s < 60 -> "$s s ago"
            s < 3_600 -> "${s / 60} min ago"
            else -> "${s / 3_600} h ago"
        }
    }

    // ------------------------------------------------------------ Set up and pair a messaging app

    /** The steps the web app gives for making a bot, plain, one per row. */
    fun setupSteps(platform: String): List<String> = if (platform == "telegram") listOf(
        "Open @BotFather in Telegram.", "Send /newbot.", "Pick a display name, then a username that ends in bot.", "BotFather replies with a token. Copy it and paste it below.",
    ) else listOf(
        "Open the Discord developer portal and choose New Application.", "Open Bot, press Reset Token and copy the token.",
        "On the same page, under Privileged Gateway Intents, turn on Message Content Intent and save.", "Paste the token below.",
    )

    fun setupLabel(platform: String) = if (platform == "telegram") "Open Telegram" else "Open the Discord developer portal"
    fun setupUrl(platform: String) = if (platform == "telegram") "https://t.me/BotFather" else "https://discord.com/developers/applications"
    fun appName(platform: String) = if (platform == "telegram") "Telegram" else "Discord"
    fun inviteUrl(appId: String) = "https://discord.com/oauth2/authorize?client_id=$appId&scope=bot%20applications.commands&permissions=309237730368"

    const val TOKEN_REFUSED = "That token did not work."
    const val INTENT_OFF = "Message Content Intent looks off. Turn it on under Bot, Privileged Gateway Intents, or the bot cannot read messages."
    const val SAVED_RESTARTED = "HQ restarted with the new settings."

    fun standbyNotice(platform: String) = "Background work is paused, so HQ cannot answer on ${appName(platform)} yet. Turn it on in Background work."

    fun disconnectWords(platform: String) = ModeWords("Disconnect ${appName(platform)}?", "HQ stops answering on ${appName(platform)} and forgets the bot token. HQ restarts for a moment.", "Disconnect")
    val REPLACE_WORDS = ModeWords("Link a different account?", "The current owner loses access until the new account pairs. Continue?", "Continue")

    fun pairBlurb(platform: String) = "Send this code to your bot. It tells HQ which ${appName(platform)} account is yours."

    /** What to do with the code when there is no button to press: where to send it. */
    fun pairHelp(platform: String, rawCommand: String, hasLink: Boolean): String = pairText(platform, rawCommand.replace(' ', '\u00A0'), hasLink)

    private fun pairText(platform: String, command: String, hasLink: Boolean): String = when {
        platform == "telegram" && hasLink -> "Tap the button to open your bot and press Start. Or send it $command yourself."
        platform == "telegram" -> "Open your bot in Telegram and send it $command."
        else -> "Open a direct message with your bot in Discord and send $command."
    }

    fun pairWaiting(left: String) = "Waiting for the message. The code works for $left."
    const val PAIR_EXPIRED = "The code ran out. Get a new one."

    /** The sentence under a failed pairing or setup call: the server's own words when it gave some. */
    fun chatAppFailure(status: Int, serverText: String?): String = when {
        status == 0 -> "HQ did not reply. Try again."
        status == 409 -> serverText ?: "That is set outside the app. Change it where it was set."
        else -> serverText ?: TOKEN_REFUSED
    }

    // ------------------------------------------------------------ Connect Claude

    data class Grant(val id: String, val client: String, val scope: String, val since: String)

    fun scopeWords(scope: String) = when (scope) { "read_only" -> "Read only"; "tasks" -> "Tasks"; "full" -> "Full"; else -> scope }

    fun grants(c: JSONObject?, now: Instant, zone: java.time.ZoneId = java.time.ZoneId.systemDefault()): List<Grant> = liveGrants(c, now).map {
        Grant(it.optString("id"), it.text("label") ?: it.text("client_name") ?: "A client", scopeWords(it.optString("grant_scope")), dayOf(time(it, "created_at"), zone))
    }

    /** Whether new connections are open, for how long, and when it closes on its own (local HH:mm). */
    data class Window(val open: Boolean, val left: String, val closesAt: String)

    fun window(c: JSONObject?, now: Instant, zone: java.time.ZoneId = java.time.ZoneId.systemDefault()): Window {
        val until = time(c, "armed_until")?.takeIf { it.isAfter(now) } ?: return Window(false, "", "")
        return Window(true, clock(Duration.between(now, until).seconds), until.atZone(zone).toLocalTime().format(java.time.format.DateTimeFormatter.ofPattern("HH:mm")))
    }

    val CLAUDE_OFF_WORDS = ModeWords("Disconnect Claude everywhere?", "Every Claude connected to this HQ loses access now, and new connections stay closed until you open them again.", "Disconnect")
    val SIGN_OUT_ALL_WORDS = ModeWords("Sign out everywhere?", "Every other device is signed out, and so is any sign-in link you made earlier. Claude connections end too. HQ restarts for a moment.", "Sign out")

    /** The name HQ gives the one sign-in that "Sign out everywhere" leaves behind. */
    const val ROTATE_LABEL = "Sign out everywhere"

    /** A device sign-in code as it is read out: ten characters as two groups of five. */
    fun groupCode(code: String): String = if (code.length == 10) code.chunked(5).joinToString("-") else code

    /** The helper line under a sign-in code. */
    fun codeHelp(left: String) = "Works once, for $left. Enter the code on the other device."

    // ------------------------------------------------------------ Other devices

    data class Device(val id: String, val label: String, val line: String, val current: Boolean)

    fun devices(d: JSONObject?, now: Instant = Instant.now(), zone: java.time.ZoneId = java.time.ZoneId.systemDefault()): List<Device> = d.arr("devices").items().map {
        val signed = dayOf(time(it, "created_at"), zone)
        val used = time(it, "last_used_at")?.let { u -> " · used ${ago(u, now)}" }.orEmpty()
        Device(it.optString("id"), it.text("label") ?: "A device", (if (signed.isEmpty()) "Signed in" else "Signed in $signed") + used, it.optBoolean("current"))
    }

    // ------------------------------------------------------------ Spending limits

    enum class BudgetTone { Ok, Warning, Exceeded }

    /** One budget as the Spending page draws it: its name and what it covers, how much is used, a bar, and where it stands. */
    data class BudgetCard(val title: String, val covers: String, val used: String, val fraction: Float, val stand: String, val tone: BudgetTone)

    fun budgetCards(b: JSONObject?, now: Instant): List<BudgetCard> = b.arr("budgets").items().map {
        val spent = numOrNull(it, "spent_usd") ?: 0.0
        val limit = numOrNull(it, "limit_usd")
        val pct = numOrNull(it, "pct") ?: if (limit != null && limit > 0) spent / limit * PERCENT else 0.0
        val tone = when (it.optString("state")) { "warning", "warn" -> BudgetTone.Warning; "exceeded", "blocked" -> BudgetTone.Exceeded; else -> BudgetTone.Ok }
        val state = when (tone) { BudgetTone.Ok -> "Within budget"; BudgetTone.Warning -> "Past the alert threshold"; BudgetTone.Exceeded -> "Limit reached" }
        val resets = numOrNull(it, "resets_at")?.let { at -> ", resets in ${span((at - now.epochSecond) / SECONDS_PER_HOUR)}" }.orEmpty()
        val period = when (it.optString("period")) { "day" -> "daily"; "week" -> "weekly"; "month" -> "monthly"; else -> it.optString("period") }
        val action = it.optString("action").takeIf { a -> a.isNotBlank() }?.let { a -> ", then $a" }.orEmpty()
        BudgetCard(
            it.optString("name").ifBlank { "Limit" }.replaceFirstChar(Char::uppercase),
            it.optString("scope").ifBlank { "global" } + ", " + period + action,
            "${UsageWords.usd(spent)} of ${UsageWords.usd(limit)} (${Math.round(pct)}%)",
            (pct / PERCENT).coerceIn(0.0, 1.0).toFloat(),
            state + resets,
            tone,
        )
    }

    /** The empty Budgets page: nothing is capped, unless HQ also holds a cap for each background run. */
    fun noBudgets(b: JSONObject?): String =
        if ((numOrNull(b, "background_run_usd") ?: 0.0) > 0.0) "No budgets are set." else "No budgets are set, so nothing is limited."

    /** Hours as the board writes them: under an hour "<1h", under a day "9h", then "3 days". */
    fun span(hours: Double): String = when {
        !hours.isFinite() || hours < 1 -> "<1h"
        hours < HOURS_PER_DAY -> "${Math.round(hours)}h"
        else -> Math.round(hours / HOURS_PER_DAY).coerceAtLeast(1).let { d -> "$d day${if (d == 1L) "" else "s"}" }
    }

    /** The month so far, where it ends, and when it resets; one line saying so when nothing has been spent yet. Null when the route gave no month. */
    data class Forecast(val rows: List<FactRow>, val note: String?)

    fun forecast(f: JSONObject?): Forecast? {
        val m = f.obj("month") ?: return null
        val spent = numOrNull(m, "spent_this_period") ?: 0.0
        if (numOrNull(m, "rate_per_hour") == null && spent <= 0.0) return Forecast(emptyList(), "No spend recorded yet, so there is nothing to forecast.")
        val rough = if (m.optString("confidence") == "low") "Not much history yet, so this is a rough guess." else null
        return Forecast(
            listOf(
                FactRow("Spent so far", UsageWords.usd(spent)),
                FactRow("Projected month end", UsageWords.usd(numOrNull(m, "projected_period_total")), rough),
                FactRow("Resets in", numOrNull(m, "hours_to_reset")?.let(::span) ?: "-"),
            ),
            null,
        )
    }

    private fun numOrNull(o: JSONObject?, name: String): Double? = o?.takeIf { it.has(name) && !it.isNull(name) }?.optDouble(name)?.takeIf { it.isFinite() }

    /** What a budget covers, when it is not everything: "global" says nothing, "provider:openrouter" says which. */
    private fun scopeOf(scope: String): String? = when {
        scope == "global" || scope.isBlank() -> null
        scope.startsWith("provider:") -> "Provider ${scope.removePrefix("provider:")}"
        scope.startsWith("model:") -> "Model ${scope.removePrefix("model:")}"
        scope.startsWith("origin:") -> "From ${scope.removePrefix("origin:")}"
        else -> scope
    }

    private fun stateWords(s: String) = when (s) { "ok", "within", "" -> "within the limit"; "warn", "warning" -> "close to the limit"; "blocked", "exceeded" -> "limit reached"; else -> s }
}
