package example.hq.spike.capture

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

/** What the Capture screen makes: a note, a task, or a draft added to the open chat. */
enum class CaptureMode(val label: String) { Note("Note"), Task("Task"), Chat("Chat") }

/** The choices kept for Capture and the widgets: what a share opens on, and whether Needs you may show titles. */
object CapturePrefs {
    private const val FILE = "capture"
    private const val SHARE_OPENS = "share_opens"
    private const val LAST_MODE = "last_mode"
    private const val SHOW_TITLES = "show_titles"
    const val LAST = "last"

    private fun prefs(c: Context) = c.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    /** "note", "task" or "last". */
    fun shareOpens(c: Context): String = prefs(c).getString(SHARE_OPENS, CaptureMode.Note.name.lowercase()) ?: CaptureMode.Note.name.lowercase()
    fun setShareOpens(c: Context, v: String) { prefs(c).edit().putString(SHARE_OPENS, v).apply() }
    fun rememberMode(c: Context, m: CaptureMode) { prefs(c).edit().putString(LAST_MODE, m.name.lowercase()).apply() }
    fun lastMode(c: Context): CaptureMode = modeOf(prefs(c).getString(LAST_MODE, null)) ?: CaptureMode.Note
    fun showTitles(c: Context): Boolean = prefs(c).getBoolean(SHOW_TITLES, false)
    fun setShowTitles(c: Context, on: Boolean) { prefs(c).edit().putBoolean(SHOW_TITLES, on).apply() }

    fun modeOf(name: String?): CaptureMode? = CaptureMode.entries.firstOrNull { it.name.equals(name, ignoreCase = true) }

    /** Where a share opens: the mode the intent asked for, else the owner's setting, where "last" is the last mode used. */
    fun startMode(asked: String?, setting: String, last: CaptureMode): CaptureMode =
        modeOf(asked) ?: if (setting == LAST) last else modeOf(setting) ?: CaptureMode.Note
}

/** What another app shared: a title and some text, often a link. */
object CaptureWords {
    private val URL = Regex("https?://[^\\s<>\"']+")
    private const val TITLE_MAX = 80
    private const val ASSIGNEES_MAX = 20

    data class Preview(val title: String, val where: String)

    fun link(text: String?): String? = URL.find(text.orEmpty())?.value?.trimEnd('.', ',', ')', ';', ':')

    /** The card above the fields for a shared link: the page's title if the app sent one, and where it is without the scheme. */
    fun preview(subject: String?, text: String?): Preview? {
        val url = link(text) ?: return null
        val uri = try { java.net.URI(url) } catch (e: java.net.URISyntaxException) { return null }
        val host = uri.host?.removePrefix("www.") ?: return null
        return Preview(subject?.trim()?.takeIf { it.isNotEmpty() } ?: host, host + (uri.rawPath ?: "").trimEnd('/'))
    }

    /** The shared subject becomes the title; with none, the first line of text that is not a link. */
    fun titleFor(subject: String?, text: String?): String {
        subject?.trim()?.takeIf { it.isNotEmpty() }?.let { return it.take(TITLE_MAX) }
        val line = text.orEmpty().lineSequence().map { it.trim() }.firstOrNull { it.isNotEmpty() && link(it) != it } ?: return ""
        return line.take(TITLE_MAX)
    }

    /** The title a note is saved under: what was typed, else the first line of what was written. */
    fun noteTitle(title: String, body: String): String =
        title.trim().ifEmpty { body.lineSequence().map { it.trim().trimStart('#', ' ') }.firstOrNull { it.isNotEmpty() }.orEmpty() }.take(TITLE_MAX).ifEmpty { "Shared note" }

    /** "hq, mobile" or "hq mobile" into names, each once, at most twenty. */
    fun assignees(typed: String): List<String> = typed.split(',', ' ', '\n').map { it.trim().trimStart('@') }.filter { it.isNotEmpty() }.distinct().take(ASSIGNEES_MAX)

    /** The text a draft in the open chat starts with: what was written, then the link if it is not already in it. */
    fun chatDraft(title: String, body: String): String = listOf(title.trim(), body.trim()).filter { it.isNotEmpty() }.distinct().joinToString("\n\n")

    fun taskBody(id: String, title: String, notes: String, priority: String?, due: String?, assignees: List<String>): JSONObject {
        val b = JSONObject().put("title", title.trim()).put("space_id", "personal").put("initiative", "Inbox").put("created_by", "operator").put("external_id", id)
        if (notes.isNotBlank()) b.put("description", notes.trim())
        priority?.let { b.put("priority", it) }
        due?.let { b.put("due_date", it) }
        if (assignees.isNotEmpty()) b.put("assignees", JSONArray(assignees))
        return b
    }

    fun noteBody(folder: String, title: String, body: String): JSONObject = JSONObject().put("folder", folder).put("title", noteTitle(title, body)).put("content", body)
}
