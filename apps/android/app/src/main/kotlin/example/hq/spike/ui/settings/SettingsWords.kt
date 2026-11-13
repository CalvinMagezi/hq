package example.hq.spike.ui.settings

import example.hq.spike.R
import example.hq.spike.ui.shell.Words
import java.time.Duration
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeParseException
import example.hq.spike.ui.usage.ChainRow
import example.hq.spike.ui.usage.UsageWords
import example.hq.spike.update.UpdateState
import org.json.JSONArray
import org.json.JSONObject

/** What each Settings route answered; a null is a route that did not answer, so its rows say so instead of guessing. */
data class SettingsSnapshot(
    val settings: JSONObject? = null,
    val chatApps: JSONObject? = null,
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
    /** Where the GitHub connection stands (/api/github). Null from an HQ without HQ Coder. */
    val github: JSONObject? = null,
)

/** Keys on this phone, Speech model and Connected tools are sections inside Keys, Assistant and Computers; their old names live on in [SettingsRoutes]. */
enum class SettingsPage { Home, Safety, Keys, Messaging, Devices, Spending, About, Background, KeepRunning, Notifications, Origins, Computers, CaptureWidgets, SetupApp, PairApp, Technical, Noticed, Update, Assistant, Calendar, Contacts, Suggests }

/** The Settings row that opens the GitHub connection. */
const val HQ_CODER_ROW = "github"

/** The Settings row that opens What HQ suggests; its line is fixed, it needs nothing from HQ to be worded. */
const val SUGGESTS_ROW = "suggests"

/** The Settings rows that open Tasks and Helpers, which are off the bar. */
const val WORK_TASKS_ROW = "work_tasks"
const val WORK_HELPERS_ROW = "work_helpers"

data class HomeRow(val page: SettingsPage?, val key: String, val title: String, val status: String, val warn: Boolean = false)

data class HomeGroup(val title: String, val rows: List<HomeRow>, val note: String? = null, val lead: String? = null)

/** A line on a read-only page: a title with its value, and a sentence that says what it means. */
data class FactRow(val title: String, val value: String, val text: String? = null, val good: Boolean = false, val opens: SettingsPage? = null)

typealias SilenceRow = SettingsWords.SilenceRow
typealias Silences = SettingsWords.Silences

/** The words of the Settings board, built from what the server sent. Pure, so each line can be tested from a real response. */
object SettingsWords {
    private fun notAnswered(w: Words) = w.s(R.string.settings_no_answer_yet)
    private const val PERCENT = 100.0
    private const val SECONDS_PER_HOUR = 3_600.0
    private const val HOURS_PER_DAY = 24.0
    private val RESET_DAY: java.time.format.DateTimeFormatter get() = java.time.format.DateTimeFormatter.ofPattern("d MMM", java.util.Locale.getDefault())

    fun yesNo(w: Words, v: Boolean) = w.s(if (v) R.string.noticed_yes else R.string.noticed_no)
    fun onOff(w: Words, v: Boolean) = w.s(if (v) R.string.settings_update_toggle_on else R.string.settings_update_toggle_off)

    private fun JSONObject?.obj(name: String): JSONObject? = this?.optJSONObject(name)
    private fun JSONObject?.arr(name: String): JSONArray? = this?.optJSONArray(name)
    private fun JSONArray?.items(): List<JSONObject> = if (this == null) emptyList() else (0 until length()).mapNotNull { optJSONObject(it) }
    private fun JSONObject?.text(name: String): String? = this?.takeIf { has(name) && !isNull(name) }?.optString(name)?.takeIf { it.isNotBlank() }

    /** Keys for search or other tools, not for an AI service: they are listed but not counted as "keys" on the home line. */
    private val TOOL_KEYS = setOf("Brave Search")

    fun configuredKeys(s: JSONObject?): List<String> = s.arr("provider_keys").items().filter { it.optBoolean("configured") && it.optString("name") !in TOOL_KEYS }.map { it.optString("name") }

    /** The parts of AI service and keys. The keys sealed on this phone never depend on HQ, so they show even when HQ does not answer (this is where a key is added when HQ will not start). */
    enum class KeysPart { Hq, Phone }

    fun keysParts(settings: JSONObject?): List<KeysPart> = if (settings == null) listOf(KeysPart.Phone) else listOf(KeysPart.Hq, KeysPart.Phone)

    fun keysStatus(w: Words, s: JSONObject?, providers: JSONObject? = null): String {
        if (s == null) return notAnswered(w)
        // The AI HQ uses is the first one in its chain, whichever keys are stored: a stored key is not a choice.
        val services = UsageWords.providers(s, providers)
        if (services.isNotEmpty()) return w.n(R.plurals.settings_services_count, services.size, services.first(), services.size)
        val keys = configuredKeys(s)
        if (keys.isEmpty()) return w.s(R.string.settings_no_key_yet)
        return w.n(R.plurals.settings_keys_count, keys.size, keys.first(), keys.size)
    }

    private fun app(w: Words, chat: JSONObject?, name: String): String = w.s(if (chat.obj(name)?.let { it.optBoolean("enabled") && it.optBoolean("token_set") } == true) R.string.settings_state_on else R.string.settings_state_off)

    fun messagingStatus(w: Words, chat: JSONObject?): String = if (chat == null) notAnswered(w) else w.s(R.string.settings_messaging_status, app(w, chat, "telegram"), app(w, chat, "discord"))

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
        return example.hq.spike.data.Instants.parse(t)
    }

    private fun epoch(n: Long): Instant = if (n >= MILLIS_FROM) Instant.ofEpochMilli(n) else Instant.ofEpochSecond(n)

    private fun dayOf(t: Instant?, zone: java.time.ZoneId): String = t?.atZone(zone)?.format(java.time.format.DateTimeFormatter.ofPattern("MMM d", java.util.Locale.getDefault())).orEmpty()

    /** "3 h ago": how long before [now], for a "used" line. */
    fun ago(w: Words, t: Instant?, now: Instant): String {
        if (t == null) return ""
        val minutes = Duration.between(t, now).toMinutes().coerceAtLeast(0)
        return when {
            minutes < 1 -> w.s(R.string.age_just_now)
            minutes < 60 -> w.s(R.string.settings_ago_min, minutes)
            minutes < 24 * 60 -> w.s(R.string.age_hours, minutes / 60)
            else -> w.s(R.string.age_days, minutes / (24 * 60))
        }
    }

    fun clock(seconds: Long): String = String.format(java.util.Locale.ROOT, "%d:%02d", seconds / 60, seconds % 60)

    fun devicesStatus(w: Words, d: JSONObject?): String {
        if (d == null) return notAnswered(w)
        val n = d.arr("devices").items().size
        return if (n == 0) w.s(R.string.settings_none_signed_in) else w.s(R.string.settings_n_signed_in, n)
    }

    /** "2,860" for the About page, or null when this HQ does not say how many notes there are (an older build). */
    fun notesCount(snap: SettingsSnapshot): String? {
        val v = snap.vault ?: return null
        if (!v.has("notes") || v.isNull("notes")) return null
        return v.optLong("notes", -1L).takeIf { it >= 0 }?.let { String.format(java.util.Locale.ROOT, "%,d", it) }
    }

    private fun money(v: Double) = "$" + String.format(java.util.Locale.ROOT, "%.2f", v)

    fun spendingStatus(w: Words, b: JSONObject?): String {
        if (b == null) return notAnswered(w)
        val all = b.arr("budgets").items()
        // The whole-spend budget (scope global), daily if there is one; a budget for one provider or model is not today's total.
        val head = all.firstOrNull { it.optString("scope") == "global" && it.optString("period") == "day" } ?: all.firstOrNull { it.optString("scope") == "global" }
        if (head == null) return if (all.isEmpty()) w.s(R.string.settings_no_limit_set) else w.n(R.plurals.spend_limits_set, all.size, all.size)
        val period = when (head.optString("period")) { "day" -> w.s(R.string.settings_period_today); "week" -> w.s(R.string.settings_period_week); "month" -> w.s(R.string.settings_period_month); else -> "" }
        return w.s(R.string.settings_spent_of, money(head.optDouble("spent_usd")), money(head.optDouble("limit_usd")), period).trim()
    }

    /** Notifications turn amber only when something needs doing: Android blocks them, or a channel for what needs the owner is off. A channel the owner turned off on purpose (App updates) is a choice. */
    fun notificationsWarn(allowed: Boolean, needsOn: Boolean, updatesOn: Boolean): Boolean = !allowed || !needsOn || !updatesOn

    /** [rows] kept to the pages of This phone and in its order. */
    fun inPhoneOrder(rows: List<HomeRow>): List<HomeRow> = PHONE_PAGES.mapNotNull { p -> rows.firstOrNull { it.page == p } }

    /** The pages the view model puts in "This phone" (read from Android, so they are not built here); the placement test counts them as on Home. */
    val PHONE_PAGES = listOf(SettingsPage.Background, SettingsPage.Notifications, SettingsPage.Assistant, SettingsPage.Calendar, SettingsPage.Contacts, SettingsPage.CaptureWidgets)

    /** The paired server's row: the state first, so the 40 character cut can only shorten the host; amber while signed out. */
    fun serverRow(w: Words, host: String, signedOut: Boolean): HomeRow =
        HomeRow(SettingsPage.Devices, "phone", w.s(R.string.settings_server_hq_row), w.s(if (signedOut) R.string.settings_server_row_out else R.string.settings_server_row_in, host), warn = signedOut)

    /** The longest a Home state line may be; a longer one is cut rather than wrapped into a second line. */
    const val STATE_MAX = 40

    private fun fit(s: String) = if (s.length <= STATE_MAX) s else s.take(STATE_MAX - 1).trimEnd() + "\u2026"

    /** What Updates says on Home, and whether that wants a look. [ready] is the version that waits, if one does. */
    fun updatesHome(w: Words, state: UpdateState, installed: String, ready: String?): Pair<String, Boolean> = when {
        state is UpdateState.Checking -> w.s(R.string.settings_updates_checking) to false
        state is UpdateState.Off -> w.s(R.string.settings_updates_off) to false
        ready != null -> w.s(R.string.settings_updates_ready, ready) to false
        state is UpdateState.InstalledWithdrawn || state is UpdateState.Problem || state is UpdateState.Failed || state is UpdateState.NeedsNewerAndroid -> w.s(R.string.settings_updates_look) to true
        installed.isBlank() -> w.s(R.string.settings_updates_checking) to false
        else -> w.s(R.string.settings_updates_version, installed) to false
    }

    /** Background work's line: on or off, then how many Keep running checks need a fix. */
    fun backgroundHome(w: Words, active: Boolean, toFix: Int): String = when {
        toFix <= 0 -> backgroundWords(w, active)
        active -> w.s(R.string.settings_background_on_fix, toFix)
        else -> w.s(R.string.settings_background_off_fix, toFix)
    }

    /** The computers paired with HQ: the list HQ keeps, without `native`, which is HQ's own machine and not a pairing. */
    fun pairedComputers(s: JSONObject?): Int = s.obj("agent_host").arr("hosts")?.let { a -> (0 until a.length()).count { a.optString(it) !in PHONE_HOSTS } } ?: 0

    private val PHONE_HOSTS = setOf("native", "local")

    private fun computersHome(w: Words, s: JSONObject?): String {
        if (s == null) return notAnswered(w)
        val n = pairedComputers(s)
        return if (n == 0) w.s(R.string.settings_computers_none) else w.n(R.plurals.settings_computers_state, n, n)
    }

    /**
     * Settings Home: six named groups. [phone] is what Android says (Background work, Notifications, Phone assistant, Calendar, Capture and widgets),
     * read by the view model; the rest comes from HQ. One state line per row, 40 characters or fewer, state first.
     */
    fun home(
        w: Words, snap: SettingsSnapshot, now: Instant = Instant.now(), phone: List<HomeRow> = emptyList(), loaded: Boolean = true, backup: BackupFacts = BackupFacts(),
        strip: String? = null, server: HomeRow? = null, updates: Pair<String, Boolean> = w.s(R.string.settings_updates_checking) to false,
    ): List<HomeGroup> = withChecking(w, loaded, listOf(
        HomeGroup(
            w.s(R.string.settings_group_work), listOf(
                HomeRow(null, WORK_TASKS_ROW, w.s(R.string.row_work_tasks), w.s(R.string.trail_work_tasks)),
                HomeRow(null, WORK_HELPERS_ROW, w.s(R.string.row_work_helpers), w.s(R.string.trail_work_helpers)),
            ),
        ),
        HomeGroup(
            w.s(R.string.settings_group_ai), listOf(
                HomeRow(SettingsPage.Keys, "key", w.s(R.string.row_ai_keys), keysStatus(w, snap.settings, snap.providers)),
                HomeRow(SettingsPage.Spending, "gauge", w.s(R.string.row_spending), spendingStatus(w, snap.budgets)),
            ),
        ),
        HomeGroup(
            w.s(R.string.settings_group_chats), listOf(
                HomeRow(SettingsPage.Messaging, "chat", w.s(R.string.row_messaging), messagingStatus(w, snap.chatApps)),
                HomeRow(SettingsPage.Devices, "phone", w.s(R.string.devices_title), devicesStatus(w, snap.devices)),
                HomeRow(SettingsPage.Computers, "computer", w.s(R.string.settings_page_computers), computersHome(w, snap.settings)),
                HomeRow(null, HQ_CODER_ROW, w.s(R.string.row_github), githubStatus(w, snap.github), warn = githubWarns(snap.github)),
            ) + listOfNotNull(server),
            note = claudeViaLaptop(w),
            lead = strip,
        ),
        HomeGroup(w.s(R.string.settings_group_phone), phone),
        HomeGroup(
            w.s(R.string.settings_group_safety), listOf(
                HomeRow(SettingsPage.Safety, "shield", w.s(R.string.row_safety), if (snap.settings == null) notAnswered(w) else w.s(R.string.connect_scope_read_only_title)),
                HomeRow(SettingsPage.Suggests, SUGGESTS_ROW, w.s(R.string.row_suggests), w.s(R.string.trail_suggests)),
            ),
        ),
        HomeGroup(
            w.s(R.string.settings_group_data), listOf(
                HomeRow(SettingsPage.About, "arch", w.s(R.string.settings_backup_title), BackupWords.homeLine(w, backup, now), warn = BackupWords.warns(backup, now)),
                HomeRow(SettingsPage.Update, "update", w.s(R.string.upd_page_title), updates.first, warn = updates.second),
                HomeRow(SettingsPage.Technical, "technical", w.s(R.string.settings_technical_title), w.s(R.string.settings_technical_home)),
            ),
        ),
    )).map { g -> g.copy(rows = g.rows.map { it.copy(status = fit(it.status)) }) }

    /** The one card above the groups, by the precedence the board sets: a pulled build, then Android stopping HQ, then the backup; the plain Update card only when none of those shows. */
    enum class HomeCard { Withdrawn, KeepStops, Backup, Update, None }

    fun homeCard(withdrawn: Boolean, keepToFix: Int, backupWarns: Boolean, updateReady: Boolean): HomeCard = when {
        withdrawn -> HomeCard.Withdrawn
        keepToFix > 0 -> HomeCard.KeepStops
        backupWarns -> HomeCard.Backup
        updateReady -> HomeCard.Update
        else -> HomeCard.None
    }

    /** Until the first read has finished, the rows that come from HQ say so, rather than "No answer yet". */
    private fun withChecking(w: Words, loaded: Boolean, groups: List<HomeGroup>): List<HomeGroup> =
        if (loaded) groups else groups.map { g -> if (g.title == w.s(R.string.settings_group_phone)) g else g.copy(rows = g.rows.map { if (it.page in LOCAL_PAGES || it.key in FIXED_ROWS) it else it.copy(status = w.s(R.string.settings_checking)) }) }

    /** Rows whose line never comes from HQ, so it has nothing to check. */
    private val FIXED_ROWS = setOf(WORK_TASKS_ROW, WORK_HELPERS_ROW, SUGGESTS_ROW)

    /** Pages whose line comes from this phone, not from HQ. */
    private val LOCAL_PAGES = setOf(SettingsPage.About, SettingsPage.Update, SettingsPage.Technical)

    /** Who GitHub is connected to, or the step that is left; what HQ Coder is, while HQ has not said. */
    fun githubStatus(w: Words, g: JSONObject?): String = when (g?.optString("standing")) {
        "connected" -> w.s(R.string.github_connected_as, g.text("owner").orEmpty())
        "needs_install" -> w.s(R.string.trail_github_install)
        "reconnect" -> w.s(R.string.trail_github_reconnect)
        "not_connected" -> w.s(R.string.trail_github_off)
        else -> w.s(R.string.trail_github_default)
    }

    fun githubWarns(g: JSONObject?): Boolean = g?.optString("standing") in setOf("needs_install", "reconnect")

    fun aboutStatus(w: Words, h: JSONObject?): String = h?.text("version")?.let { w.s(R.string.settings_hq_version, it) } ?: if (h == null) notAnswered(w) else w.s(R.string.settings_hq_running)

    // ------------------------------------------------------------ Safety and limits (read only, four tabs)

    enum class SafetyTab(@androidx.annotation.StringRes val label: Int) { Safety(R.string.settings_tab_safety), Time(R.string.settings_tab_time), Helpers(R.string.place_helpers), Tools(R.string.settings_tab_tools) }

    /** `inbox.fleet.helper_policy`: what helpers may ask the owner. [id] is HQ's word for it. */
    enum class HelperPolicy(val id: String, @androidx.annotation.StringRes val title: Int, @androidx.annotation.StringRes val about: Int) {
        Nothing("nothing", R.string.settings_helper_policy_nothing, R.string.settings_helper_policy_nothing_about),
        LosesWork("loses_work", R.string.settings_helper_policy_loses, R.string.settings_helper_policy_loses_about),
        Everything("everything", R.string.settings_helper_policy_everything, R.string.settings_helper_policy_everything_about),
    }

    /** The standing rule as HQ reports it; null when HQ does not say or names one this app does not know. */
    fun helperPolicy(s: JSONObject?): HelperPolicy? {
        val id = s?.optJSONObject("inbox")?.optJSONObject("fleet")?.optString("helper_policy", "").orEmpty()
        return HelperPolicy.entries.firstOrNull { it.id == id }
    }

    private fun secs(w: Words, v: Int): String = when {
        v <= 0 -> w.s(R.string.settings_update_toggle_off)
        v % 3600 == 0 -> w.s(R.string.settings_hours_n, v / 3600)
        v % 60 == 0 -> w.s(R.string.settings_minutes_n, v / 60)
        else -> w.s(R.string.settings_seconds_n, v)
    }

    private fun sandbox(w: Words, v: String): String = when (v) {
        "required" -> w.s(R.string.settings_sandbox_required)
        "best_effort" -> w.s(R.string.settings_sandbox_best_effort)
        "process" -> w.s(R.string.settings_sandbox_process)
        "off", "none", "" -> w.s(R.string.settings_update_toggle_off)
        else -> v.replaceFirstChar(Char::uppercase)
    }

    /** The computer a helper runs on: the phone itself is "native" in the config. */
    private fun computerName(w: Words, host: String) = if (host == "native" || host == "local") w.s(R.string.settings_this_phone) else host

    /** A count that opens its list, unless there is nothing in it to open. */
    private fun countRow(title: String, n: Int, page: SettingsPage, text: String? = null) = FactRow(title, "$n", text, opens = if (n > 0) page else null)

    /** The web addresses allowed to open HQ, one per row. */
    fun originRows(s: JSONObject?): List<String> = s.obj("safety").arr("web_allowed_origins").let { a -> if (a == null) emptyList() else (0 until a.length()).map { a.optString(it) }.filter { it.isNotBlank() } }

    /** Each tool server HQ can call: its name, where it lives, and when it is used. */
    fun toolRows(w: Words, s: JSONObject?): List<FactRow> = s.obj("integrations").arr("remote_mcp").items().map {
        val lines = listOfNotNull(it.text("host"), w.s(R.string.settings_tool_live_only).takeIf { _ -> it.optBoolean("live_user_turn_only") })
        FactRow(it.text("name") ?: w.s(R.string.settings_tool_fallback), "", lines.joinToString("\n").ifBlank { null })
    }

    /** The computers helpers can run on; the default one says so. */
    fun computerRows(w: Words, s: JSONObject?): List<FactRow> {
        val host = s.obj("agent_host")
        val default = host.text("default_host")
        val a = host.arr("hosts") ?: return emptyList()
        return (0 until a.length()).mapNotNull { i -> a.optString(i).takeIf { it.isNotBlank() } }.map { FactRow(computerName(w, it), "", if (it == default) w.s(R.string.settings_default) else null) }
    }

    fun safetyRows(w: Words, s: JSONObject?, tab: SafetyTab): List<FactRow> {
        if (s == null) return emptyList()
        val safety = s.obj("safety")
        val limits = s.obj("limits")
        val host = s.obj("agent_host")
        val integ = s.obj("integrations")
        return when (tab) {
            SafetyTab.Safety -> listOf(
                FactRow(w.s(R.string.settings_safe_box), sandbox(w, safety.text("bash_sandbox").orEmpty()), w.s(R.string.settings_safe_box_text)),
                FactRow(w.s(R.string.settings_cmd_net), yesNo(w, safety?.optBoolean("bash_network", true) ?: true), w.s(R.string.settings_cmd_net_text)),
                FactRow(w.s(R.string.settings_skills_approval), yesNo(w, safety?.optBoolean("skills_write_approval") == true), w.s(R.string.settings_skills_approval_text)),
                countRow(w.s(R.string.settings_origins_count), safety.arr("web_allowed_origins")?.length() ?: 0, SettingsPage.Origins, w.s(R.string.settings_origins_count_text)),
            )
            SafetyTab.Time -> listOf(
                FactRow(w.s(R.string.settings_time_longest), secs(w, limits?.optInt("chat_turn_timeout_secs") ?: 0)),
                FactRow(w.s(R.string.settings_time_ack), secs(w, limits?.optInt("turn_ack_timeout_secs") ?: 0)),
                FactRow(w.s(R.string.settings_time_bg_days), w.s(R.string.settings_days_n, limits?.optInt("background_turn_max_days") ?: 0)),
                FactRow(w.s(R.string.settings_time_checkin), secs(w, limits?.optInt("background_progress_secs") ?: 0)),
            )
            SafetyTab.Helpers -> listOf(
                FactRow(w.s(R.string.settings_helper_default), host.text("default_host")?.let { computerName(w, it) } ?: w.s(R.string.settings_none_set)),
                countRow(w.s(R.string.settings_section_computers), host.arr("hosts")?.length() ?: 0, SettingsPage.Computers),
                FactRow(w.s(R.string.settings_helper_safe_box), sandbox(w, host.text("sandbox_mode").orEmpty())),
                FactRow(w.s(R.string.settings_helper_idle), host?.optInt("idle_reap_hours")?.let { if (it <= 0) w.s(R.string.sheet_never) else w.s(R.string.settings_hours_n, it) } ?: w.s(R.string.sheet_never)),
                FactRow(w.s(R.string.settings_safety_say_yes), yesNo(w, host?.optBoolean("drive_new_watches") == true)),
                FactRow(w.s(R.string.settings_helper_checkin), host?.optInt("driver_checkin_minutes")?.let { w.s(R.string.settings_minutes_n, it) } ?: w.s(R.string.settings_key_not_set)),
                FactRow(w.s(R.string.settings_helper_budget), "${host?.optInt("driver_nudge_budget") ?: 0}"),
                FactRow(w.s(R.string.settings_helper_no_progress), (host?.optInt("driver_no_progress_limit") ?: 0).let { n -> w.n(R.plurals.settings_no_progress_after, n, n) }, w.s(R.string.settings_helper_no_progress_text)),
            )
            SafetyTab.Tools -> listOf(
                countRow(w.s(R.string.settings_tools_connected), integ.arr("remote_mcp")?.length() ?: 0, SettingsPage.Computers),
                FactRow(w.s(R.string.settings_tools_search), integ.text("searxng_host") ?: w.s(R.string.settings_built_in)),
                FactRow(w.s(R.string.settings_tools_disk), onOff(w, integ?.optBoolean("disk_watchdog_enabled") == true)),
            )
        }
    }

    // ------------------------------------------------------------ Add a key

    /** The button a key row gets: only the three providers HQ can check have one, and a key set by the environment cannot be changed here. */
    data class KeyAction(val id: String, val label: String)

    fun keyAction(w: Words, s: JSONObject?, name: String, set: Boolean, source: String?): KeyAction? {
        if (source == "env") return null
        val id = s.arr("provider_keys").items().firstOrNull { it.optString("name") == name }?.text("id") ?: return null
        return KeyAction(id, w.s(if (set) R.string.settings_replace else R.string.settings_add))
    }

    data class KeyWords(val title: String, val body: String)

    fun keyWords(w: Words, name: String, replace: Boolean) = KeyWords(
        w.s(if (replace) R.string.phone_settings_key_replace_title else R.string.phone_settings_key_add_title, name),
        w.s(R.string.settings_key_body, name),
    )

    fun keyRefused(w: Words) = w.s(R.string.settings_key_refused)
    fun keyNoAnswer(w: Words) = w.s(R.string.settings_key_no_answer)
    fun keyEnv(w: Words) = w.s(R.string.settings_key_env)
    fun keyOther(w: Words) = w.s(R.string.settings_key_other)
    fun keyNote(w: Words) = w.s(R.string.phone_settings_key_note)

    /** What the sheet says when a key was not saved: the key was refused, the environment owns it, or HQ did not answer (not blamed on the key). */
    fun keyFailure(w: Words, status: Int): String = when {
        status == 0 || status >= HTTP_SERVER -> keyNoAnswer(w)
        status == HTTP_CONFLICT -> keyEnv(w)
        status == HTTP_BAD_KEY -> keyRefused(w)
        else -> keyOther(w)
    }

    private const val HTTP_SERVER = 500
    private const val HTTP_CONFLICT = 409
    private const val HTTP_BAD_KEY = 400

    fun keySaved(w: Words, name: String) = w.s(R.string.phone_settings_key_saved, name)

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
    fun aiRows(w: Words, snap: SettingsSnapshot): List<FactRow> = UsageWords.chain(snap.settings, snap.providers).map { r ->
        val status = UsageWords.statusWords(w, UsageWords.providerRow(snap.providers, r.backend)?.optString("status"))
        FactRow("${r.position} · ${r.provider}", w.s(if (r.first) R.string.settings_first_choice else R.string.settings_if_down), listOf(r.model.substringAfterLast('/'), status).filter { it.isNotBlank() }.joinToString(" · ").ifBlank { null })
    }

    // ------------------------------------------------------------ Spending by AI service

    /** What each AI service in use has left and what it cost: credits for a plan, a balance where the provider gives one, else HQ's own record. */
    fun serviceRows(w: Words, snap: SettingsSnapshot): List<FactRow> {
        val chain = UsageWords.chain(snap.settings, snap.providers)
        val services: List<ChainRow> = if (chain.isNotEmpty()) chain.distinctBy { it.provider } else snap.providers.arr("backends").items().map {
            val name = it.optString("backend")
            ChainRow(name, UsageWords.providerName(null, null, name), "", 0)
        }
        return services.map { r -> if (r.provider == UsageWords.COPILOT) copilotRow(w, snap.copilot) else providerRow(w, r, UsageWords.providerRow(snap.providers, r.backend)) }
    }

    private fun copilotRow(w: Words, c: JSONObject?): FactRow {
        val q = c.obj("quota")
        val left = q?.takeIf { c?.optBoolean("active") == true }?.optDouble("remaining", Double.NaN)?.takeIf { it.isFinite() }
        val total = q?.optDouble("entitlement", Double.NaN)?.takeIf { it.isFinite() }
        val value = when {
            left != null && c?.has("burn") == true -> if (total != null) w.s(R.string.settings_credits_left_of, UsageWords.compact(left), UsageWords.compact(total)) else w.s(R.string.settings_credits_left, UsageWords.compact(left))
            q != null && c?.optBoolean("active") == true -> w.s(R.string.settings_not_metered)
            c?.optBoolean("unavailable") == true || c?.has("error") == true -> w.s(R.string.settings_not_readable)
            else -> w.s(R.string.settings_not_measured)
        }
        val reset = q.text("reset_at")?.let { iso(it)?.let { t -> w.s(R.string.settings_resets_on, RESET_DAY.format(t.atZone(java.time.ZoneId.systemDefault()))) } }
        return FactRow(UsageWords.COPILOT, value, listOfNotNull(w.s(R.string.settings_credits_on_plan), reset).joinToString(" · "))
    }

    private fun providerRow(w: Words, r: ChainRow, row: JSONObject?): FactRow {
        val reading = row.obj("provider")
        val ledger = row.obj("ledger")
        fun num(o: JSONObject?, name: String): Double? = o?.takeIf { it.has(name) && !it.isNull(name) }?.optDouble(name)?.takeIf { it.isFinite() }
        val left = UsageWords.leftText(reading)
        val spend = reading.obj("spend") ?: ledger
        val today = UsageWords.todayAmount(row)
        val value = when {
            left != null -> w.s(R.string.settings_left, left)
            today != null -> w.s(R.string.settings_today_amount, UsageWords.usd(today))
            else -> UsageWords.statusWords(w, row?.optString("status")).ifBlank { w.s(R.string.settings_not_measured) }
        }
        val windows = listOfNotNull(
            today?.let { w.s(R.string.settings_window_today, UsageWords.usd(it)) },
            num(spend, "week")?.let { w.s(R.string.settings_window_week, UsageWords.usd(it)) },
            num(spend, "month")?.let { w.s(R.string.settings_window_month, UsageWords.usd(it)) },
        )
        val fromLedger = reading == null && ledger != null
        return FactRow(r.provider, value, (windows.joinToString(" · ") + if (fromLedger && windows.isNotEmpty()) " " + w.s(R.string.settings_own_record) else "").ifBlank { null })
    }

    fun activeModel(w: Words, s: JSONObject?): String = s.obj("model").text("active") ?: s.obj("model").text("default_model") ?: w.s(R.string.settings_key_not_set)

    fun fallbackRows(w: Words, s: JSONObject?): List<Pair<String, String>> {
        val b = s.obj("backends") ?: return emptyList()
        if (!b.optBoolean("configured", true)) return listOf(w.s(R.string.settings_fb_whichever) to w.s(R.string.settings_fb_no_order))
        val primary = b.text("primary")
        val fallbacks = b.arr("fallbacks")?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty()
        val first = primary?.let { listOf("1 · $it" to w.s(R.string.settings_fb_primary)) } ?: listOf(w.s(R.string.settings_fb_one_none) to w.s(R.string.settings_fb_no_ai))
        return first + if (fallbacks.isEmpty()) listOf(w.s(R.string.settings_fb_two_none) to w.s(R.string.settings_fb_no_fallback)) else fallbacks.mapIndexed { i, f -> "${i + 2} · $f" to w.s(R.string.settings_fb_fallback) }
    }

    // ------------------------------------------------------------ Messaging apps

    data class AppCard(val name: String, val on: Boolean, val line: String, val bot: String?, val owner: String, val running: Boolean = false, val linked: Boolean = false)

    fun appCard(w: Words, chat: JSONObject?, key: String, name: String, now: Instant = Instant.now()): AppCard {
        val a = chat.obj(key)
        val on = a?.let { it.optBoolean("enabled") && it.optBoolean("token_set") } == true
        val relay = a.obj("relay")
        val status = relay.text("status")
        // `current` is false when the status file was written before this HQ started (an earlier Active run), so nothing is answering now.
        val current = relay?.optBoolean("current", true) != false
        val running = on && current && (status == "connected" || status == "running" || status == "online")
        val line = when {
            !on -> w.s(R.string.settings_update_toggle_off)
            !current -> w.s(R.string.settings_app_not_connected)
            running -> w.s(R.string.settings_running) + (time(relay, "last_seen")?.let { " \u00b7 " + w.s(R.string.settings_seen, seen(w, it, now)) }.orEmpty())
            status != null -> status.replaceFirstChar(Char::uppercase)
            else -> w.s(R.string.note_save_saved)
        } + if (a?.optBoolean("needs_restart") == true) " · " + w.s(R.string.settings_restart_needed) else ""
        val bot = a.obj("bot").text("username")?.let { if (key == "telegram") "@$it" else it }
        val paired = a.obj("owner")?.optBoolean("paired") == true
        return AppCard(name, on, line, bot, w.s(if (paired) R.string.settings_linked else R.string.settings_not_linked), running, paired)
    }

    // ------------------------------------------------------------ Background work

    /** The three lines under "Right now": hq itself, and each chat app as far as the run mode lets it be connected. */
    fun rightNow(w: Words, snap: SettingsSnapshot, active: Boolean, restarting: Boolean): List<FactRow> {
        val hq = when {
            restarting -> FactRow("HQ", w.s(R.string.hub_restarting_state))
            snap.health != null -> FactRow("HQ", w.s(R.string.settings_running), good = true)
            else -> FactRow("HQ", notAnswered(w))
        }
        return listOf(hq, chatAppRow(w, snap.chatApps, "telegram", "Telegram", active), chatAppRow(w, snap.chatApps, "discord", "Discord", active))
    }

    private fun chatAppRow(w: Words, chat: JSONObject?, key: String, name: String, active: Boolean): FactRow {
        val a = chat.obj(key)
        val relay = a.obj("relay")
        val status = relay.text("status")
        val value = when {
            a == null || !(a.optBoolean("enabled") && a.optBoolean("token_set")) -> w.s(R.string.settings_not_set_up)
            !active -> w.s(R.string.settings_paused)
            relay?.optBoolean("current", true) == false -> w.s(R.string.settings_app_not_connected)
            status == "connected" -> w.s(R.string.state_connected)
            status == "error" -> w.s(R.string.settings_error)
            else -> w.s(R.string.settings_app_not_connected)
        }
        return FactRow(name, value)
    }

    data class ModeWords(val title: String, val body: String, val confirm: String)

    fun backgroundWords(w: Words, active: Boolean) = w.s(if (active) R.string.settings_background_on else R.string.settings_background_off)

    fun restartWords(w: Words) = ModeWords(w.s(R.string.settings_restart_title), w.s(R.string.settings_restart_body), w.s(R.string.hub_restart))

    /** The text "Copy diagnostics" puts on the clipboard: the build, what runs, where it serves, and what HQ says about itself. */
    fun diagnostics(build: String, active: Boolean, health: JSONObject?, port: Int): String = buildString {
        appendLine("AgentHQ hub build $build")
        appendLine("Run mode: ${if (active) "Active: relays and scheduler on" else "Standby: web app only"}. Serving this device on 127.0.0.1:$port")
        appendLine("Health: ${health?.toString() ?: "no answer"}")
    }

    /** What the owner reads, and then copies, on Technical details. Versions, states and counts only: never a title, a name, an address or a key. */
    data class DetailFacts(val version: String, val build: String, val android: String, val backgroundOn: Boolean, val hq: String)

    /** The text of the Copy details sheet: what the app is, and the "What HQ noticed" numbers as English keys. */
    fun copyDetails(w: Words, f: DetailFacts, noticed: example.hq.spike.measure.CounterSnapshot): String = buildString {
        appendLine(w.s(R.string.settings_details_app, f.version, f.build, f.android))
        appendLine(w.s(R.string.settings_details_bg, onOff(w, f.backgroundOn)))
        appendLine(w.s(R.string.settings_details_hq, f.hq))
        append(example.hq.spike.measure.Noticed.block(noticed))
    }

    /** What the dialog says before the run mode changes: what stops or starts, and that hq restarts for a moment. */
    fun modeDialog(w: Words, toActive: Boolean) = if (toActive) {
        ModeWords(w.s(R.string.settings_mode_start_title), w.s(R.string.settings_mode_start_body), w.s(R.string.helpers_start))
    } else {
        ModeWords(w.s(R.string.settings_mode_pause_title), w.s(R.string.settings_mode_pause_body), w.s(R.string.dl_btn_pause))
    }

    /** "12 s ago", "3 min ago", "2 h ago": how long since a relay last said it was there. */
    fun seen(w: Words, t: Instant, now: Instant): String {
        val s = Duration.between(t, now).seconds.coerceAtLeast(0)
        return when {
            s < 60 -> w.s(R.string.settings_ago_sec, s)
            s < 3_600 -> w.s(R.string.settings_ago_min, s / 60)
            else -> w.s(R.string.age_hours, s / 3_600)
        }
    }

    // ------------------------------------------------------------ Set up and pair a messaging app

    /** The steps the web app gives for making a bot, plain, one per row. */
    fun setupSteps(w: Words, platform: String): List<String> = if (platform == "telegram") listOf(
        R.string.settings_tg_step1, R.string.settings_tg_step2, R.string.settings_tg_step3, R.string.settings_tg_step4,
    ).map(w::s) else listOf(
        R.string.settings_dc_step1, R.string.settings_dc_step2, R.string.settings_dc_step3, R.string.settings_dc_step4,
    ).map(w::s)

    fun setupLabel(w: Words, platform: String) = w.s(if (platform == "telegram") R.string.settings_open_telegram else R.string.settings_open_discord_portal)
    fun setupUrl(platform: String) = if (platform == "telegram") "https://t.me/BotFather" else "https://discord.com/developers/applications"
    fun appName(platform: String) = if (platform == "telegram") "Telegram" else "Discord"
    fun inviteUrl(appId: String) = "https://discord.com/oauth2/authorize?client_id=$appId&scope=bot%20applications.commands&permissions=309237730368"

    fun tokenRefused(w: Words) = w.s(R.string.settings_token_refused)
    fun intentOff(w: Words) = w.s(R.string.settings_intent_off)
    fun savedRestarted(w: Words) = w.s(R.string.settings_saved_restarted)

    fun standbyNotice(w: Words, platform: String) = w.s(R.string.settings_standby_notice, appName(platform))

    fun pairBlurb(w: Words, platform: String) = w.s(R.string.settings_pair_blurb, appName(platform))

    /** What to do with the code when there is no button to press: where to send it. */
    fun pairHelp(w: Words, platform: String, rawCommand: String, hasLink: Boolean): String = pairText(w, platform, rawCommand.replace(' ', '\u00A0'), hasLink)

    private fun pairText(w: Words, platform: String, command: String, hasLink: Boolean): String = when {
        platform == "telegram" && hasLink -> w.s(R.string.settings_pair_tg_link, command)
        platform == "telegram" -> w.s(R.string.settings_pair_tg, command)
        else -> w.s(R.string.settings_pair_dc, command)
    }

    fun pairWaiting(w: Words, left: String) = w.s(R.string.settings_pair_waiting, left)
    fun pairExpired(w: Words) = w.s(R.string.settings_pair_expired)

    /** The sentence under a failed pairing or setup call: the server's own words when it gave some. */
    fun chatAppFailure(w: Words, status: Int, serverText: String?): String = when {
        status == 0 -> keyNoAnswer(w)
        status == 409 -> serverText ?: w.s(R.string.settings_set_outside)
        else -> serverText ?: tokenRefused(w)
    }

    fun claudeViaLaptop(w: Words) = w.s(R.string.settings_claude_via_laptop)

    /** The name HQ gives the one sign-in that "Sign out everywhere" leaves behind. */
    const val ROTATE_LABEL = "Sign out everywhere"

    /** A device sign-in code as it is read out: ten characters as two groups of five. */
    fun groupCode(code: String): String = if (code.length == 10) code.chunked(5).joinToString("-") else code

    /** The helper line under a sign-in code. */
    fun codeHelp(w: Words, left: String) = w.s(R.string.settings_code_help, left)

    // ------------------------------------------------------------ Other devices

    data class Device(val id: String, val label: String, val line: String, val current: Boolean)

    fun devices(w: Words, d: JSONObject?, now: Instant = Instant.now(), zone: java.time.ZoneId = java.time.ZoneId.systemDefault()): List<Device> = d.arr("devices").items().map {
        val signed = dayOf(time(it, "created_at"), zone)
        val used = time(it, "last_used_at")?.let { u -> " · " + w.s(R.string.settings_used, ago(w, u, now)) }.orEmpty()
        Device(it.optString("id"), it.text("label") ?: w.s(R.string.settings_a_device), (if (signed.isEmpty()) w.s(R.string.settings_label_signed_in) else w.s(R.string.settings_signed_in_on, signed)) + used, it.optBoolean("current"))
    }

    // ------------------------------------------------------------ Spending limits

    enum class BudgetTone { Ok, Warning, Exceeded }

    /** One budget as the Spending page draws it: its name and what it covers, how much is used, a bar, and where it stands. */
    data class BudgetCard(val title: String, val covers: String, val used: String, val fraction: Float, val stand: String, val tone: BudgetTone, val resetNote: String? = null)

    /** "openrouter" reads "OpenRouter"; a name nobody mapped is only capitalised. */
    fun serviceName(id: String): String = SERVICE_NAMES[id.lowercase()] ?: id.replaceFirstChar(Char::uppercase)

    private val SERVICE_NAMES = mapOf("openrouter" to "OpenRouter", "anthropic" to "Anthropic", "openai" to "OpenAI", "google" to "Google", "ollama" to "Ollama", "copilot" to "GitHub Copilot")

    /** "provider:openrouter" reads "OpenRouter only", "model:anthropic/claude-x" reads "claude-x only", and a global budget reads "All AI spending". */
    fun scopeTitle(w: Words, scope: String): String = when {
        scope == "global" || scope.isBlank() -> w.s(R.string.spend_scope_all)
        scope.startsWith("provider:") -> w.s(R.string.spend_scope_service, serviceName(scope.removePrefix("provider:")))
        scope.startsWith("model:") -> w.s(R.string.spend_scope_model, scope.removePrefix("model:").substringAfterLast('/'))
        scope.startsWith("origin:") -> w.s(R.string.spend_scope_origin, scope.removePrefix("origin:"))
        else -> scope
    }

    fun budgetCards(w: Words, b: JSONObject?, now: Instant, zone: ZoneId = ZoneId.systemDefault()): List<BudgetCard> = b.arr("budgets").items().map {
        val spent = numOrNull(it, "spent_usd") ?: 0.0
        val limit = numOrNull(it, "limit_usd")
        val pct = numOrNull(it, "pct") ?: if (limit != null && limit > 0) spent / limit * PERCENT else 0.0
        val tone = when (it.optString("state")) { "warning", "warn" -> BudgetTone.Warning; "exceeded", "blocked" -> BudgetTone.Exceeded; else -> BudgetTone.Ok }
        val resetsAt = numOrNull(it, "resets_at")?.toLong()
        val span = resetsAt?.let { at -> span(w, (at - now.epochSecond).toDouble() / SECONDS_PER_HOUR) }
        val stand = when (tone) {
            BudgetTone.Ok -> if (span == null) w.s(R.string.spend_stand_ok) else w.s(R.string.spend_stand_ok_resets, span)
            BudgetTone.Warning -> if (span == null) w.s(R.string.spend_stand_warn) else w.s(R.string.spend_stand_warn_resets, span)
            BudgetTone.Exceeded -> if (span == null) w.s(R.string.spend_stand_exceeded) else w.s(R.string.spend_stand_exceeded_resets, span)
        }
        val period = it.optString("period")
        val action = it.optString("action")
        val covers = when (period) {
            "day" -> when (action) { "block" -> R.string.spend_covers_day_block; "notify", "alert", "warn" -> R.string.spend_covers_day_notify; else -> R.string.spend_covers_day }
            "week" -> when (action) { "block" -> R.string.spend_covers_week_block; "notify", "alert", "warn" -> R.string.spend_covers_week_notify; else -> R.string.spend_covers_week }
            "month" -> when (action) { "block" -> R.string.spend_covers_month_block; "notify", "alert", "warn" -> R.string.spend_covers_month_notify; else -> R.string.spend_covers_month }
            else -> null
        }
        val midnightUtc = period == "day" && resetsAt != null && resetsAt % 86_400L == 0L
        BudgetCard(
            scopeTitle(w, it.optString("scope")),
            covers?.let(w::s).orEmpty(),
            "${UsageWords.usd(spent)} of ${UsageWords.usd(limit)} (${Math.round(pct)}%)",
            (pct / PERCENT).coerceIn(0.0, 1.0).toFloat(),
            stand,
            tone,
            if (midnightUtc) w.s(R.string.spend_reset_utc_here, w.clock(Instant.ofEpochSecond(resetsAt!!), zone)) else null,
        )
    }

    /** The empty Budgets page: nothing is capped, unless HQ also holds a cap for each background run. */
    fun noBudgets(w: Words, b: JSONObject?): String =
        w.s(if ((numOrNull(b, "background_run_usd") ?: 0.0) > 0.0) R.string.settings_no_budgets else R.string.settings_no_budgets_none_limited)

    /** Hours the way the person reads them: "under 1 h", "9 h", then "3 days". */
    fun span(w: Words, hours: Double): String = when {
        !hours.isFinite() || hours < 1 -> w.s(R.string.spend_span_under_hour)
        hours < HOURS_PER_DAY -> w.s(R.string.spend_span_hours, Math.round(hours).toInt())
        else -> Math.round(hours / HOURS_PER_DAY).coerceAtLeast(1).toInt().let { d -> w.n(R.plurals.spend_span_days, d, d) }
    }

    /** The month so far, where it ends, and when it resets; one line saying so when nothing has been spent yet. Null when the route gave no month. */
    data class Forecast(val rows: List<FactRow>, val note: String?)

    fun forecast(w: Words, f: JSONObject?): Forecast? {
        val m = f.obj("month") ?: return null
        val spent = numOrNull(m, "spent_this_period") ?: 0.0
        if (numOrNull(m, "rate_per_hour") == null && spent <= 0.0) return Forecast(emptyList(), w.s(R.string.spend_forecast_none))
        val rough = if (m.optString("confidence") == "low") w.s(R.string.spend_forecast_rough) else null
        return Forecast(
            listOf(
                FactRow(w.s(R.string.spend_forecast_spent), UsageWords.usd(spent)),
                FactRow(w.s(R.string.spend_forecast_total), UsageWords.usd(numOrNull(m, "projected_period_total")), rough),
                FactRow(w.s(R.string.spend_forecast_resets), numOrNull(m, "hours_to_reset")?.let { h -> span(w, h) } ?: "-"),
            ),
            null,
        )
    }

    private fun numOrNull(o: JSONObject?, name: String): Double? = o?.takeIf { it.has(name) && !it.isNull(name) }?.optDouble(name)?.takeIf { it.isFinite() }

    data class SilenceRow(val topic: String, val silenced: Boolean, val until: Instant?)
    data class Silences(val pause: SilenceRow, val topics: List<SilenceRow>)

    private val SILENCE_TIME_FORMAT: java.time.format.DateTimeFormatter
        get() = java.time.format.DateTimeFormatter.ofPattern("EEE h:mm a", java.util.Locale.getDefault())

    fun silences(o: JSONObject): Silences {
        fun row(obj: JSONObject?) = SilenceRow(
            topic = obj?.optString("topic").orEmpty(),
            silenced = obj?.optBoolean("silenced", false) == true,
            until = time(obj, "until"),
        )
        val pause = row(o.optJSONObject("pause"))
        val topics = o.arr("topics").items().map(::row)
        return Silences(pause, topics)
    }

    fun silenceLine(w: Words, row: SilenceRow, zone: ZoneId = ZoneId.systemDefault()): String {
        if (!row.silenced) return w.s(R.string.silence_on)
        val until = row.until ?: return w.s(R.string.silence_off)
        return w.s(R.string.silence_off_until, until.atZone(zone).format(SILENCE_TIME_FORMAT))
    }

    fun pauseLine(w: Words, pause: SilenceRow, zone: ZoneId = ZoneId.systemDefault()): String? {
        if (!pause.silenced) return null
        val until = pause.until ?: return w.s(R.string.silence_paused)
        return w.s(R.string.silence_paused_until, until.atZone(zone).format(SILENCE_TIME_FORMAT))
    }
}
