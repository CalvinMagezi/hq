package example.hq.spike.ui.notes

import java.time.Instant
import java.time.format.DateTimeParseException
import org.json.JSONArray
import org.json.JSONObject

/** One thing the notes check wants looked at: where it is, what it says, and the repair HQ has for it. */
data class HealthItem(
    val note: String,
    val text: String,
    val detail: String?,
    /** The repair for this item, when HQ has one; the id the server named it by. */
    val fixId: String?,
    /** HQ is certain (true) or it is a good guess (false); meaningless without a [fixId]. */
    val sure: Boolean,
)

/**
 * The notes check as the card and its page read it. Built from `GET /api/vault/health`, which is the
 * same report the web app shows, so the two say the same numbers.
 */
data class VaultHealth(
    val dead: Int = 0,
    val ambiguous: Int = 0,
    val anchors: Int = 0,
    val duplicates: Int = 0,
    val frontmatter: Int = 0,
    /** How many fixes HQ is certain about. */
    val certain: Int = 0,
    /** The latest repair, while it can still be undone: its run id and how many things it fixed. */
    val undoRun: String? = null,
    val undoFixes: Int = 0,
    val items: List<HealthItem> = emptyList(),
) {
    /** Things a person could fix. Notes nothing links to are not counted: they are not damage. */
    val problems: Int get() = dead + ambiguous + anchors + duplicates + frontmatter

    /** Whether the card has anything to say: something to look at, or a fix that can be undone. */
    val worthShowing: Boolean get() = problems > 0 || undoRun != null
}

object VaultHealthParse {
    /** A repair can be undone from the card for this long. The same day as on the web. */
    const val UNDO_WINDOW_MS = 24L * 60 * 60 * 1000

    private val LINK_FIXES = setOf("stale_path", "renamed_note", "near_miss", "same_folder")

    /** The check from the server's answer; null when no check has run yet, or the answer is not one. */
    fun from(json: JSONObject?, nowMs: Long = System.currentTimeMillis()): VaultHealth? {
        if (json == null || !json.optBoolean("checked", false)) return null
        val report = json.optJSONObject("report") ?: return null
        val counts = report.optJSONObject("counts") ?: return null
        val links = counts.optJSONObject("links")
        val fixes = fixes(json.optJSONArray("fixes"))
        return VaultHealth(
            dead = links?.optInt("dead") ?: 0,
            ambiguous = links?.optInt("ambiguous") ?: 0,
            anchors = links?.optInt("broken_anchors") ?: 0,
            duplicates = counts.optInt("duplicates"),
            frontmatter = counts.optInt("frontmatter"),
            certain = json.optJSONObject("fix_counts")?.optInt("a") ?: 0,
            undoRun = undoable(json.optJSONArray("repairs"), nowMs)?.first,
            undoFixes = undoable(json.optJSONArray("repairs"), nowMs)?.second ?: 0,
            items = items(report, fixes),
        )
    }

    /** The latest repair as (run id, fixes) when it is recent, not taken back and did something. */
    internal fun undoable(repairs: JSONArray?, nowMs: Long): Pair<String, Int>? {
        val latest = repairs?.optJSONObject(0) ?: return null
        if (latest.text("undone_at") != null) return null
        val fixes = latest.optInt("fixes")
        if (fixes == 0) return null
        val at = try { Instant.parse(latest.optString("ran_at")).toEpochMilli() } catch (e: DateTimeParseException) { return null }
        return if (nowMs - at < UNDO_WINDOW_MS) latest.optString("run_id") to fixes else null
    }

    /** The text at [key], or null when it is missing, empty or a JSON null (which `optString` would read as the word "null"). */
    private fun JSONObject.text(key: String): String? = if (isNull(key)) null else optString(key).ifEmpty { null }

    private data class Fix(val id: String, val kind: String, val tier: String, val note: String, val before: String)

    private fun fixes(a: JSONArray?): List<Fix> = (0 until (a?.length() ?: 0)).mapNotNull { i ->
        a?.optJSONObject(i)?.let { Fix(it.optString("id"), it.optString("kind"), it.optString("tier"), it.optString("note"), it.optString("before")) }
    }

    private fun items(report: JSONObject, fixes: List<Fix>): List<HealthItem> {
        val out = mutableListOf<HealthItem>()
        fun each(key: String, build: (JSONObject) -> HealthItem?) {
            val a = report.optJSONArray(key) ?: return
            for (i in 0 until a.length()) a.optJSONObject(i)?.let(build)?.let(out::add)
        }
        fun linkFix(note: String, link: String): Fix? = fixes.firstOrNull { it.kind in LINK_FIXES && it.note == note && it.before == link }
        each("dead_links") { d ->
            val note = d.optString("note"); val link = d.optString("link")
            val fix = linkFix(note, link)
            HealthItem(note, link, d.text("did_you_mean"), fix?.id, fix?.tier == "a")
        }
        each("ambiguous_links") { a ->
            val note = a.optString("note"); val link = a.optString("link")
            val fix = linkFix(note, link)
            HealthItem(note, link, null, fix?.id, fix?.tier == "a")
        }
        each("broken_anchors") { b -> HealthItem(b.optString("note"), b.optString("link"), null, null, false) }
        each("frontmatter") { f ->
            val note = f.optString("note")
            val fix = fixes.firstOrNull { it.kind == "nested_frontmatter" && it.note == note }
            HealthItem(note, note.substringAfterLast('/').removeSuffix(".md"), null, fix?.id, fix?.tier == "a")
        }
        each("duplicates") { g ->
            val notes = g.optJSONArray("notes") ?: return@each null
            val names = (0 until notes.length()).map { notes.optString(it) }
            val fix = fixes.firstOrNull { it.kind == "merge_duplicate" && it.note in names }
            HealthItem(names.firstOrNull().orEmpty(), names.joinToString(", ") { it.substringAfterLast('/').removeSuffix(".md") }, null, fix?.id, fix?.tier == "a")
        }
        return out
    }
}
