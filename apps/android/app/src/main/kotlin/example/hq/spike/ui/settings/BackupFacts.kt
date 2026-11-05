package example.hq.spike.ui.settings

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
)

enum class BackupState { Server, CouldNotCheck, Off, Failed, Waiting, Overdue, Recent }

object BackupWords {
    enum class Tone { Good, Warn }

    data class Card(val tone: Tone, val title: String, val body: String, val offerSetUp: Boolean)

    private const val DEFAULT_EVERY_HOURS = 24
    private const val DEFAULT_KEEP = 7
    private const val MAX_MESSAGE = 160
    private const val STALE_FACTOR = 2L
    private const val OFF_BODY = "If the app is uninstalled or the phone is lost, your notes and tasks go with it. Backup today runs from the config file."

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

    private const val HOUR_MS = 3_600_000L
    private const val WEEK_MS = 7 * 24 * HOUR_MS

    /** First match wins: server HQ, could not check, not set up, failed, waiting, overdue, recent. */
    fun state(f: BackupFacts, now: Instant): BackupState {
        val c = f.config
        if (f.server) return BackupState.Server
        if (!f.readable) return BackupState.CouldNotCheck
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
        val off = f.copyOffPhoneMs
        return off == null || now.toEpochMilli() - off > WEEK_MS
    }

    /** The home row and the card warn together, from the same answer. */
    fun warns(f: BackupFacts, now: Instant): Boolean = when (state(f, now)) {
        BackupState.Server -> false
        BackupState.Off, BackupState.Recent -> onlyHere(f, now)
        BackupState.Waiting -> false
        else -> true
    }

    fun homeLine(f: BackupFacts, now: Instant): String = when (state(f, now)) {
        BackupState.Server -> "Backups run on your server HQ"
        BackupState.CouldNotCheck -> "Could not check your backups"
        BackupState.Failed -> "The last backup did not finish"
        BackupState.Waiting -> "Waiting for the first backup"
        BackupState.Overdue -> if (f.newestMs == null) "No backup has been made" else "The last backup is old"
        BackupState.Off -> if (onlyHere(f, now)) "Your notes are only on this phone" else "Backups are off"
        BackupState.Recent -> if (onlyHere(f, now)) "Your notes are only on this phone" else "Last backup ${ago(Duration.between(Instant.ofEpochMilli(f.newestMs ?: 0), now).toHours().coerceAtLeast(0))}"
    }

    fun card(f: BackupFacts, now: Instant): Card {
        val c = f.config
        val state = state(f, now)
        val warn = warns(f, now)
        val tone = if (warn) Tone.Warn else Tone.Good
        val plan = c?.let { "A new one is made every ${it.everyHours} hours and the newest ${it.keep} are kept." }.orEmpty()
        return when (state) {
            BackupState.Server -> Card(tone, "Backups run on your server HQ", "This phone does not make them.", false)
            BackupState.CouldNotCheck -> Card(tone, "Could not check your backups", "HQ's backup files are there, but this app could not read them.", false)
            BackupState.Off -> Card(tone, if (warn) "Your notes are only on this phone" else "Backup is not set up", OFF_BODY, offerSetUp = true)
            BackupState.Failed -> Card(tone, "The last backup did not finish", f.status?.message.orEmpty().take(MAX_MESSAGE).ifBlank { "HQ did not say why." }, false)
            BackupState.Waiting -> Card(tone, "Waiting for the first backup", "No backup has finished yet. $plan", false)
            BackupState.Overdue -> {
                val why = when {
                    f.newestMs != null -> "Last backup ${ago(Duration.between(Instant.ofEpochMilli(f.newestMs), now).toHours().coerceAtLeast(0))}. $plan"
                    f.dirOutside -> "The backup folder in the config is outside this app's files."
                    c?.keyFile != null && f.status == null -> "No backup has been made. Check the folder and the key file in the config."
                    else -> "No backup has been made. $plan"
                }
                Card(tone, if (f.newestMs == null) "No backup has been made" else "The last backup is old", why, false)
            }
            BackupState.Recent -> {
                val hours = Duration.between(Instant.ofEpochMilli(f.newestMs ?: 0), now).toHours().coerceAtLeast(0)
                // The file counts belong to the backup the status file names, so they are shown only when that is the newest one.
                val ofNewest = f.status?.takeIf { f.newestName != null && it.path.substringAfterLast('/') == f.newestName }
                val files = ofNewest?.takeIf { it.files > 0 }?.let { ", ${it.files} files" }.orEmpty()
                val left = ofNewest?.takeIf { it.leftOut > 0 }?.let { " ${it.leftOut} files were left out." }.orEmpty()
                Card(tone, if (warn) "Your notes are only on this phone" else "Backup is on", "Last backup ${ago(hours)}$files. $plan$left", false)
            }
        }
    }

    private fun ago(hours: Long) = when {
        hours < 1 -> "just now"
        hours < 24 -> "$hours h ago"
        else -> "${hours / 24} d ago"
    }
}
