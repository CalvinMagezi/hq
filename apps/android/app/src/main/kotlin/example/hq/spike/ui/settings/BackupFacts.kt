package example.hq.spike.ui.settings

import example.hq.spike.R
import example.hq.spike.data.HqClient
import example.hq.spike.ui.shell.Words
import java.time.Duration
import java.time.Instant
import org.json.JSONException
import org.json.JSONObject

/** The `backup:` block of the config file: whether HQ makes scheduled backups, and how often and how many it keeps. */
data class BackupConfig(val enabled: Boolean, val everyHours: Int, val keep: Int, val dir: String?, val keyFile: String? = null)

/** What HQ wrote in `backup-status.json` about the last scheduled backup it tried. */
data class BackupStatus(val ok: Boolean, val message: String, val files: Int, val leftOut: Int, val path: String = "")

/** [newestMs] and [newestName] are the newest backup file in the folder, however it was made. */
data class BackupFacts(
    val config: BackupConfig? = null,
    val status: BackupStatus? = null,
    val newestMs: Long? = null,
    val newestName: String? = null,
    /** False when a file that exists could not be read: that is "could not check", never "no backup". */
    val readable: Boolean = true,
    /** The config names a folder outside the app's own files, which backups never use. */
    val dirOutside: Boolean = false,
    /** When this phone first saw backups switched on, to tell "none made yet" from "none made for days". */
    val firstSeenOnMs: Long? = null,
    /** The last time a copy was saved somewhere other than this phone. */
    val copyOffPhoneMs: Long? = null,
    /** Backups run on a server HQ, not on this phone. */
    val server: Boolean = false,
    /** The server's name, for the card that says where backups run. */
    val serverName: String? = null,
    /** What the server said at `/api/backup/status`, or null when it did not say. */
    val serverReport: HqClient.ServerBackup? = null,
    /** Background work is not running on this phone, so no scheduled backup can run. */
    val paused: Boolean = false,
    /** The update channel's `saveCopy` switch: off hides Save a copy. */
    val saveCopyOn: Boolean = true,
    /** The person pressed "Got it" on the not-set-up card with Save a copy off. */
    val gotItDismissed: Boolean = false,
)

enum class BackupState { Server, CouldNotCheck, Off, Failed, Waiting, Overdue, Recent }

object BackupWords {
    enum class Tone { Good, Warn }

    enum class Action { SaveCopy, OpenBackground, GotIt, CopyDetails }

    /** [sentences] make up the body; the Home slot shows the first two. [small] and [lock] are the quiet lines under it. */
    data class Card(
        val tone: Tone,
        val title: String,
        val sentences: List<String>,
        val small: String? = null,
        val said: String? = null,
        val lock: String? = null,
        val actions: List<Action> = emptyList(),
    ) {
        val body: String get() = sentences.joinToString(" ")
    }

    /** What the Home card slot says: only when the backup row warns. */
    data class Slot(val title: String, val body: String)

    private const val DEFAULT_EVERY_HOURS = 24
    private const val DEFAULT_KEEP = 7
    private const val MAX_MESSAGE = 160
    private const val STALE_FACTOR = 2L
    private const val HOURS_PER_DAY = 24
    private const val SLOT_SENTENCES = 2
    private const val HOUR_MS = 3_600_000L
    private const val WEEK_MS = 7 * 24 * HOUR_MS

    /** Reads only the `backup:` block; everything else in the file (it holds keys) is never looked at. */
    fun parseConfig(yaml: String): BackupConfig? {
        val lines = yaml.lines()
        val start = lines.indexOfFirst { it.trimEnd() == "backup:" }
        if (start < 0) return null
        val block = lines.drop(start + 1).takeWhile { it.isBlank() || it.first().isWhitespace() }.map { it.trim() }
        fun value(key: String) = block.firstOrNull { it.startsWith("$key:") }?.substringAfter(':')?.substringBefore('#')?.trim()?.trim('"', '\'')?.ifBlank { null }
        return BackupConfig(
            enabled = value("enabled") == "true",
            everyHours = value("every_hours")?.toIntOrNull()?.takeIf { it > 0 } ?: DEFAULT_EVERY_HOURS,
            keep = value("keep")?.toIntOrNull()?.takeIf { it > 0 } ?: DEFAULT_KEEP,
            dir = value("dir"),
            keyFile = value("key_file"),
        )
    }

    fun parseStatus(json: String): BackupStatus? = try {
        val o = JSONObject(json)
        BackupStatus(o.optBoolean("ok"), o.optString("message"), o.optInt("files"), o.optInt("left_out"), o.optString("path"))
    } catch (e: JSONException) { null }

    /** First match wins: server HQ, could not check, not set up, failed, waiting, overdue, recent. */
    fun state(f: BackupFacts, now: Instant): BackupState {
        val c = f.config
        if (f.server) return BackupState.Server
        if (!f.readable || f.dirOutside) return BackupState.CouldNotCheck
        if (c == null || !c.enabled) return BackupState.Off
        if (f.status?.ok == false) return BackupState.Failed
        val limit = c.everyHours * STALE_FACTOR * HOUR_MS
        val newest = f.newestMs
        if (newest == null) {
            val seen = f.firstSeenOnMs ?: return BackupState.Waiting
            return if (now.toEpochMilli() - seen >= limit) BackupState.Overdue else BackupState.Waiting
        }
        return if (now.toEpochMilli() - newest >= limit) BackupState.Overdue else BackupState.Recent
    }

    /** Backups exist, or could, but nothing has been saved off this phone in the last week. */
    fun onlyHere(f: BackupFacts, now: Instant): Boolean {
        val s = state(f, now)
        if (s != BackupState.Recent && s != BackupState.Off) return false
        if (s == BackupState.Off && !f.saveCopyOn) return false
        val off = f.copyOffPhoneMs
        return off == null || now.toEpochMilli() - off > WEEK_MS
    }

    /** The home row and the card warn together, from the same answer. */
    fun warns(f: BackupFacts, now: Instant): Boolean = when (state(f, now)) {
        BackupState.Server, BackupState.Waiting -> false
        BackupState.Off, BackupState.Recent -> onlyHere(f, now)
        else -> true
    }

    fun homeLine(w: Words, f: BackupFacts, now: Instant): String = when (state(f, now)) {
        BackupState.Server -> w.s(R.string.bk_home_server)
        BackupState.CouldNotCheck -> w.s(R.string.bk_home_could_not)
        BackupState.Failed -> w.s(R.string.bk_home_failed)
        BackupState.Waiting -> w.s(R.string.bk_home_waiting)
        BackupState.Overdue -> f.newestMs?.let { w.s(R.string.bk_home_overdue, newestAge(w, f, now)) } ?: w.s(R.string.bk_home_overdue_none)
        BackupState.Off -> when {
            onlyHere(f, now) -> w.s(R.string.bk_home_off_here)
            f.copyOffPhoneMs != null -> w.s(R.string.bk_home_off_copy, age(w, f.copyOffPhoneMs, now))
            else -> w.s(R.string.bk_home_off)
        }
        BackupState.Recent -> if (onlyHere(f, now)) w.s(R.string.bk_home_recent_here, newestAge(w, f, now)) else w.s(R.string.bk_home_recent, newestAge(w, f, now))
    }

    /** The Home card slot: only when the backup row warns, with the first two sentences of the card. */
    fun slot(w: Words, f: BackupFacts, now: Instant): Slot? {
        if (!warns(f, now)) return null
        return Slot(w.s(R.string.bk_slot_title), card(w, f, now).sentences.take(SLOT_SENTENCES).joinToString(" "))
    }

    fun card(w: Words, f: BackupFacts, now: Instant): Card {
        val c = f.config
        val state = state(f, now)
        val tone = if (warns(f, now)) Tone.Warn else Tone.Good
        val hours = c?.everyHours ?: DEFAULT_EVERY_HOURS
        val save = if (f.saveCopyOn && state != BackupState.Server) listOf(Action.SaveCopy) else emptyList()
        val lock = w.s(R.string.bk_lock_line).takeIf { save.isNotEmpty() }
        return when (state) {
            BackupState.Server -> Card(tone, w.s(R.string.bk_title_server), listOf(serverBody(w, f, now)))
            BackupState.CouldNotCheck -> Card(
                tone, w.s(R.string.bk_title_could_not), listOf(w.s(if (f.dirOutside) R.string.bk_could_not_folder else R.string.bk_could_not_read)),
                lock = lock, actions = save + Action.CopyDetails,
            )
            BackupState.Off -> offCard(w, f, now, tone, save, lock)
            BackupState.Failed -> Card(
                tone, w.s(R.string.bk_title_failed),
                listOf(if (f.newestMs != null) w.s(R.string.bk_body_failed, newestAge(w, f, now)) else w.s(R.string.bk_body_failed_none)),
                said = f.status?.message.orEmpty().take(MAX_MESSAGE).trim().ifBlank { null }?.let { w.s(R.string.bk_said, it) } ?: w.s(R.string.bk_said_none),
                lock = lock, actions = save + Action.CopyDetails,
            )
            BackupState.Waiting -> Card(tone, w.s(R.string.bk_title_on), listOf(w.s(R.string.bk_body_waiting, hours)), lock = lock, actions = save)
            BackupState.Overdue -> overdueCard(w, f, now, tone, save, lock)
            BackupState.Recent -> recentCard(w, f, now, tone, save, lock)
        }
    }

    /** What goes in the "Copy details" sheet for whoever set up HQ: facts only, never a key, a path or a message from outside. */
    fun details(f: BackupFacts, now: Instant): String {
        val c = f.config
        return listOf(
            "state=${state(f, now)}",
            "enabled=${c?.enabled}",
            "every_hours=${c?.everyHours}",
            "keep=${c?.keep}",
            "has_dir=${c?.dir != null}",
            "has_key_file=${c?.keyFile != null}",
            "dir_outside=${f.dirOutside}",
            "readable=${f.readable}",
            "newest_hours=${f.newestMs?.let { Duration.between(Instant.ofEpochMilli(it), now).toHours() }}",
            "status_ok=${f.status?.ok}",
            "paused=${f.paused}",
        ).joinToString("\n")
    }

    private fun serverBody(w: Words, f: BackupFacts, now: Instant): String {
        val name = f.serverName.orEmpty()
        val r = f.serverReport ?: return w.s(R.string.bk_server_body, name)
        return when {
            !r.configured -> w.s(R.string.bk_server_body_none, name)
            r.failed -> w.s(R.string.bk_server_body_failed, name)
            r.lastOkMs != null -> w.s(R.string.bk_server_body_age, name, age(w, r.lastOkMs, now))
            else -> w.s(R.string.bk_server_body, name)
        }
    }

    private fun offCard(w: Words, f: BackupFacts, now: Instant, tone: Tone, save: List<Action>, lock: String?): Card = when {
        !f.saveCopyOn -> {
            val all = listOf(w.s(R.string.bk_body_off_none), w.s(R.string.bk_off_none_later))
            Card(tone, w.s(R.string.bk_title_off), if (f.gotItDismissed) all.take(1) else all, actions = if (f.gotItDismissed) emptyList() else listOf(Action.GotIt))
        }
        onlyHere(f, now) -> Card(tone, w.s(R.string.bk_title_here), listOf(w.s(R.string.bk_body_off_here), w.s(R.string.bk_off_cannot)), lock = lock, actions = save)
        else -> Card(tone, w.s(R.string.bk_title_off), listOf(w.s(R.string.bk_body_off_copy, age(w, f.copyOffPhoneMs ?: 0, now))), lock = lock, actions = save)
    }

    private fun overdueCard(w: Words, f: BackupFacts, now: Instant, tone: Tone, save: List<Action>, lock: String?): Card {
        val hours = f.config?.everyHours ?: DEFAULT_EVERY_HOURS
        val why = w.s(if (f.paused) R.string.bk_old_paused else R.string.bk_old_running)
        val actions = save + if (f.paused) Action.OpenBackground else Action.CopyDetails
        if (f.newestMs != null) return Card(tone, w.s(R.string.bk_title_old), listOf(w.s(R.string.bk_body_old, newestAge(w, f, now), hours), why), lock = lock, actions = actions)
        val limitHours = hours * STALE_FACTOR.toInt()
        val first = if (limitHours % HOURS_PER_DAY == 0) w.n(R.plurals.bk_body_none_days, limitHours / HOURS_PER_DAY, limitHours / HOURS_PER_DAY) else w.s(R.string.bk_body_none_hours, limitHours)
        val missing = f.config?.let { it.dir == null || it.keyFile == null } ?: true
        val sentences = listOfNotNull(first, w.s(R.string.bk_none_missing).takeIf { missing }, why.takeIf { f.paused }, w.s(R.string.bk_none_copy_details).takeIf { !f.paused })
        return Card(tone, w.s(R.string.bk_title_none), sentences, lock = lock, actions = actions)
    }

    private fun recentCard(w: Words, f: BackupFacts, now: Instant, tone: Tone, save: List<Action>, lock: String?): Card {
        val c = f.config
        val newestAge = newestAge(w, f, now)
        // The file counts belong to the backup the status file names, so they are shown only when that is the newest one.
        val ofNewest = f.status?.takeIf { f.newestName != null && it.path.substringAfterLast('/') == f.newestName }
        val files = ofNewest?.files?.takeIf { it > 0 }
        val left = ofNewest?.leftOut?.takeIf { it > 0 }?.let { w.n(R.plurals.bk_left_out, it, it) }
        if (onlyHere(f, now)) {
            val first = files?.let { w.n(R.plurals.bk_body_here, it, newestAge, it) } ?: w.s(R.string.bk_body_here_plain, newestAge)
            return Card(tone, w.s(R.string.bk_title_here), listOfNotNull(first, w.s(R.string.bk_here_ask).takeIf { save.isNotEmpty() }), lock = lock, actions = save)
        }
        val hours = c?.everyHours ?: DEFAULT_EVERY_HOURS
        val keep = c?.keep ?: DEFAULT_KEEP
        val first = files?.let { w.n(R.plurals.bk_body_recent, it, newestAge, it, hours, keep) } ?: w.s(R.string.bk_body_recent_plain, newestAge, hours, keep)
        return Card(
            tone, w.s(R.string.bk_title_on), listOfNotNull(first, left),
            small = f.copyOffPhoneMs?.let { w.s(R.string.bk_off_copy_small, age(w, it, now)) }, lock = lock, actions = save,
        )
    }

    private fun newestAge(w: Words, f: BackupFacts, now: Instant) = age(w, f.newestMs ?: 0, now)

    /** "just now", "5 h ago" or "2 d ago" from a time in epoch milliseconds. */
    fun age(w: Words, thenMs: Long, now: Instant): String {
        val hours = Duration.between(Instant.ofEpochMilli(thenMs), now).toHours().coerceAtLeast(0)
        return when {
            hours < 1 -> w.s(R.string.age_just_now)
            hours < HOURS_PER_DAY -> w.s(R.string.age_hours, hours)
            else -> w.s(R.string.age_days, hours / HOURS_PER_DAY)
        }
    }
}
