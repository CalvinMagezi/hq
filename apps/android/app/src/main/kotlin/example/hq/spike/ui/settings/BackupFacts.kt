package example.hq.spike.ui.settings

import java.time.Duration
import java.time.Instant
import org.json.JSONException
import org.json.JSONObject

/** The `backup:` block of the config file: whether HQ makes scheduled backups, and how often and how many it keeps. */
data class BackupConfig(val enabled: Boolean, val everyHours: Int, val keep: Int, val dir: String?)

/** What HQ wrote in `backup-status.json` about the last scheduled backup it tried. */
data class BackupStatus(val ok: Boolean, val message: String, val files: Int, val leftOut: Int)

/** [newestMs] is the newest backup file in the folder, however it was made. */
data class BackupFacts(val config: BackupConfig? = null, val status: BackupStatus? = null, val newestMs: Long? = null)

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
        )
    }

    fun parseStatus(json: String): BackupStatus? = try {
        val o = JSONObject(json)
        BackupStatus(o.optBoolean("ok"), o.optString("message"), o.optInt("files"), o.optInt("left_out"))
    } catch (e: JSONException) { null }

    fun card(f: BackupFacts, now: Instant): Card {
        val c = f.config
        if (c == null || !c.enabled) return Card(Tone.Warn, "Backup is not set up", OFF_BODY, offerSetUp = true)
        val plan = "A new one is made every ${c.everyHours} hours and the newest ${c.keep} are kept."
        val status = f.status
        if (status != null && !status.ok) return Card(Tone.Warn, "The last backup did not finish", status.message.take(MAX_MESSAGE).ifBlank { "HQ did not say why." }, offerSetUp = false)
        val newest = f.newestMs ?: return Card(Tone.Good, "Backup is on", "No backup has finished yet. $plan", offerSetUp = false)
        val hours = Duration.between(Instant.ofEpochMilli(newest), now).toHours().coerceAtLeast(0)
        val last = "Last backup ${ago(hours)}"
        if (hours >= c.everyHours * STALE_FACTOR) return Card(Tone.Warn, "The last backup is old", "$last. $plan", offerSetUp = false)
        val files = status?.takeIf { it.files > 0 }?.let { ", ${it.files} files" }.orEmpty()
        val left = status?.takeIf { it.leftOut > 0 }?.let { " ${it.leftOut} files were left out." }.orEmpty()
        return Card(Tone.Good, "Backup is on", "$last$files. $plan$left", offerSetUp = false)
    }

    private fun ago(hours: Long) = when {
        hours < 1 -> "just now"
        hours < 24 -> "$hours h ago"
        else -> "${hours / 24} d ago"
    }
}
