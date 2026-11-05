package example.hq.spike.ui.chat

import org.json.JSONObject

/** A chip on the empty chat: what HQ can do, in the person's words, and the words it puts in the composer. */
data class Suggestion(val group: String, val label: String, val text: String)

/**
 * The empty chat's chips. HQ picks them (`GET /api/capabilities`): things that work on this install, the ones the
 * person has not tried first. Until it answers, or when it has nothing ready, the fixed three stand in.
 */
object Suggestions {
    const val MAX = 4

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
                if (label.isEmpty() || text.isEmpty()) null else Suggestion(c.optString("group"), label, text)
            }
            .take(MAX)
    }

    /** HQ's own chips when it has some, else the fixed three. */
    fun shown(fromHq: List<Suggestion>?): List<Suggestion> = if (fromHq.isNullOrEmpty()) Fallback else fromHq
}
