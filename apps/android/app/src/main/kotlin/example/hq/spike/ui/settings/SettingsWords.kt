package example.hq.spike.ui.settings

import java.time.Duration
import java.time.Instant
import java.time.format.DateTimeParseException
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
)

enum class SettingsPage { Home, Safety, Keys, Messaging, Claude, Devices, Spending, About }

data class HomeRow(val page: SettingsPage?, val key: String, val title: String, val status: String, val warn: Boolean = false)

data class HomeGroup(val title: String, val rows: List<HomeRow>)

/** A line on a read-only page: a title with its value, and a sentence that says what it means. */
data class FactRow(val title: String, val value: String, val text: String? = null)

/** The words of the Settings board, built from what the server sent. Pure, so each line can be tested from a real response. */
object SettingsWords {
    private const val NOT_ANSWERED = "Not answering"

    fun yesNo(v: Boolean) = if (v) "Yes" else "No"
    fun onOff(v: Boolean) = if (v) "On" else "Off"

    private fun JSONObject?.obj(name: String): JSONObject? = this?.optJSONObject(name)
    private fun JSONObject?.arr(name: String): JSONArray? = this?.optJSONArray(name)
    private fun JSONArray?.items(): List<JSONObject> = if (this == null) emptyList() else (0 until length()).mapNotNull { optJSONObject(it) }
    private fun JSONObject?.text(name: String): String? = this?.takeIf { has(name) && !isNull(name) }?.optString(name)?.takeIf { it.isNotBlank() }

    /** Keys for search or other tools, not for an AI service: they are listed but not counted as "keys" on the home line. */
    private val TOOL_KEYS = setOf("Brave Search")

    fun configuredKeys(s: JSONObject?): List<String> = s.arr("provider_keys").items().filter { it.optBoolean("configured") && it.optString("name") !in TOOL_KEYS }.map { it.optString("name") }

    fun keysStatus(s: JSONObject?): String {
        if (s == null) return NOT_ANSWERED
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
                HomeRow(SettingsPage.Keys, "key", "AI service and keys", keysStatus(snap.settings)),
                HomeRow(SettingsPage.Messaging, "chat", "Messaging apps", messagingStatus(snap.chatApps)),
                HomeRow(SettingsPage.Claude, "plug", "Connect Claude", connectorStatus(snap.connector, now)),
                HomeRow(SettingsPage.Devices, "phone", "Other devices", devicesStatus(snap.devices)),
            ),
        ),
        HomeGroup("How HQ behaves", listOf(HomeRow(SettingsPage.Safety, "shield", "Safety and limits", if (snap.settings == null) NOT_ANSWERED else "Read only"))),
        HomeGroup(
            "Your data", listOf(
                HomeRow(SettingsPage.Spending, "gauge", "Spending limits", spendingStatus(snap.budgets)),
                HomeRow(SettingsPage.About, "arch", "About", aboutStatus(snap.health)),
            ),
        ),
    ))

    /** Until the first read has finished, the rows that come from HQ say so, rather than "Not answering". */
    private fun withChecking(loaded: Boolean, groups: List<HomeGroup>): List<HomeGroup> =
        if (loaded) groups else groups.map { g -> if (g.title == "This phone") g else g.copy(rows = g.rows.map { it.copy(status = "Checking…") }) }

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
                FactRow("Addresses allowed to open HQ", "${safety.arr("web_allowed_origins")?.length() ?: 0}", "The web addresses that may show HQ's screens."),
            )
            SafetyTab.Time -> listOf(
                FactRow("Longest time for one reply", secs(limits?.optInt("chat_turn_timeout_secs") ?: 0)),
                FactRow("How long a reply can take before it moves to the background", secs(limits?.optInt("turn_ack_timeout_secs") ?: 0)),
                FactRow("How long background work can run", "${limits?.optInt("background_turn_max_days") ?: 0} days"),
                FactRow("How often long work checks in", secs(limits?.optInt("background_progress_secs") ?: 0)),
            )
            SafetyTab.Helpers -> listOf(
                FactRow("Default computer", host.text("default_host")?.let(::computerName) ?: "None set"),
                FactRow("Computers", "${host.arr("hosts")?.length() ?: 0}"),
                FactRow("Safe box for helpers", sandbox(host.text("sandbox_mode").orEmpty())),
                FactRow("Stop idle helpers after", host?.optInt("idle_reap_hours")?.let { if (it <= 0) "Never" else "$it h" } ?: "Never"),
                FactRow("New helpers: HQ answers their questions", yesNo(host?.optBoolean("drive_new_watches") == true)),
                FactRow("How often HQ checks on helpers", host?.optInt("driver_checkin_minutes")?.let { "$it min" } ?: "Not set"),
                FactRow("How many instructions HQ may give one helper", "${host?.optInt("driver_nudge_budget") ?: 0}"),
                FactRow("Stop answering a helper that makes no progress after", "${host?.optInt("driver_no_progress_limit") ?: 0} turns", "HQ stops answering its questions for you. The helper itself keeps running."),
            )
            SafetyTab.Tools -> listOf(
                FactRow("Connected tools", "${integ.arr("remote_mcp")?.length() ?: 0}"),
                FactRow("Search server", integ.text("searxng_host") ?: "Built in"),
                FactRow("Disk space check", onOff(integ?.optBoolean("disk_watchdog_enabled") == true)),
            )
        }
    }

    // ------------------------------------------------------------ AI service and keys

    fun keyRows(s: JSONObject?): List<Triple<String, Boolean, String?>> =
        s.arr("provider_keys").items().map { Triple(it.optString("name"), it.optBoolean("configured"), it.text("source")) }

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

    fun appCard(chat: JSONObject?, key: String, name: String): AppCard {
        val a = chat.obj(key)
        val on = a?.let { it.optBoolean("enabled") && it.optBoolean("token_set") } == true
        val relay = a.obj("relay")
        val status = relay.text("status")
        // `current` is false when the status file was written before this HQ started (an earlier Active run), so nothing is answering now.
        val current = relay?.optBoolean("current", true) != false
        val line = when {
            !on -> "Off"
            !current -> "Not running yet"
            status == "connected" || status == "running" || status == "online" -> "Running"
            status != null -> status.replaceFirstChar(Char::uppercase)
            else -> "Saved"
        } + if (a?.optBoolean("needs_restart") == true) " · restart needed" else ""
        val bot = a.obj("bot").text("username")?.let { if (key == "telegram") "@$it" else it }
        val paired = a.obj("owner")?.optBoolean("paired") == true
        return AppCard(name, on, line, bot, if (paired) "Linked" else "Not linked")
    }

    // ------------------------------------------------------------ Connect Claude

    data class Grant(val id: String, val client: String, val scope: String, val since: String)

    fun scopeWords(scope: String) = when (scope) { "read_only" -> "Read only"; "tasks" -> "Tasks"; "full" -> "Full"; else -> scope }

    fun grants(c: JSONObject?, now: Instant, zone: java.time.ZoneId = java.time.ZoneId.systemDefault()): List<Grant> = liveGrants(c, now).map {
        Grant(it.optString("id"), it.text("label") ?: it.text("client_name") ?: "A client", scopeWords(it.optString("grant_scope")), dayOf(time(it, "created_at"), zone))
    }

    // ------------------------------------------------------------ Other devices

    data class Device(val id: String, val label: String, val line: String, val current: Boolean)

    fun devices(d: JSONObject?, now: Instant = Instant.now(), zone: java.time.ZoneId = java.time.ZoneId.systemDefault()): List<Device> = d.arr("devices").items().map {
        val signed = dayOf(time(it, "created_at"), zone)
        val used = time(it, "last_used_at")?.let { u -> " · used ${ago(u, now)}" }.orEmpty()
        Device(it.optString("id"), it.text("label") ?: "A device", (if (signed.isEmpty()) "Signed in" else "Signed in $signed") + used, it.optBoolean("current"))
    }

    // ------------------------------------------------------------ Spending limits

    fun budgetRows(b: JSONObject?): List<FactRow> = b.arr("budgets").items().map {
        val period = when (it.optString("period")) { "day" -> "Per day"; "week" -> "Per week"; "month" -> "Per month"; else -> it.optString("period") }
        val scope = scopeOf(it.optString("scope"))
        FactRow(it.optString("name").ifBlank { "Limit" }.replaceFirstChar(Char::uppercase), "${money(it.optDouble("spent_usd"))} of ${money(it.optDouble("limit_usd"))}", listOfNotNull(period, scope, stateWords(it.optString("state"))).joinToString(" · "))
    }

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
