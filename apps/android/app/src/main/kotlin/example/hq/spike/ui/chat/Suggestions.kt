package example.hq.spike.ui.chat

import org.json.JSONObject

/** A chip on the empty chat: what HQ can do, in the person's words, and the words it puts in the composer. */
data class Suggestion(val group: String, val label: String, val text: String, /** HQ's capability id, for Fewer like this and Never show; blank for the fixed fallback pills. */ val id: String = "")

/**
 * The empty chat's chips. HQ picks them (`GET /api/capabilities`): things that work on this install, the ones the
 * person has not tried first. Until it answers, or when it has nothing ready, the fixed three stand in.
 */
object Suggestions {
    const val MAX = 3

    /** A pill is read at a glance: at most this many characters, the last of them the ellipsis when it was cut. */
    const val LABEL_MAX = 40
    private const val ELLIPSIS = "…"
    private const val MIN_WORD_CUT = 20

    val Fallback = listOf(
        Suggestion("notes", "Summarise this note", "Summarise this note"),
        Suggestion("tasks", "What needs me today?", "What needs me today?"),
        Suggestion("notes", "Save a thought as a note", "Save a thought as a note"),
    )

    /** HQ's suggestions that are ready now, at most [MAX]; none before a model key is set. */
    fun parse(body: String): List<Suggestion> {
        val o = JSONObject(body)
        if (o.optBoolean("needs_model")) return emptyList()
        val list = o.optJSONArray("suggestions") ?: return emptyList()
        return (0 until list.length()).mapNotNull { list.optJSONObject(it) }
            .filter { it.optString("state") == "ready" }
            .mapNotNull { c ->
                val label = c.optString("says").trim()
                val text = c.optString("example").trim()
                if (label.isEmpty() || text.isEmpty()) null else Suggestion(c.optString("group"), cap(label), text, c.optString("id").trim())
            }
            .take(MAX)
    }

    /** [label] cut to [LABEL_MAX] characters at a word end where one is near, so a pill never runs on or breaks a word in half. */
    fun cap(label: String): String {
        if (label.length <= LABEL_MAX) return label
        val room = label.take(LABEL_MAX - ELLIPSIS.length)
        val atWord = room.lastIndexOf(' ').takeIf { it >= MIN_WORD_CUT }
        return (if (atWord != null && label[room.length] != ' ') room.take(atWord) else room).trimEnd(' ', ',', '.', ';', ':', '-') + ELLIPSIS
    }

    /** [list] without the pill with this capability [id]; one with no id stays. */
    fun without(list: List<Suggestion>?, id: String): List<Suggestion>? = list?.filter { it.id.isEmpty() || it.id != id }

    /** What the empty chat draws: nothing after the last HQ pill was removed, else [shown]. */
    fun visible(fromHq: List<Suggestion>?, cleared: Boolean): List<Suggestion> = if (cleared) emptyList() else shown(fromHq)

    /** HQ's own chips when it has some, else the fixed three. */
    fun shown(fromHq: List<Suggestion>?): List<Suggestion> = if (fromHq.isNullOrEmpty()) Fallback else fromHq
}
