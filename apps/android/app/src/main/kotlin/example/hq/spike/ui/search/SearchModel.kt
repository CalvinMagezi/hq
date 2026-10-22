package example.hq.spike.ui.search

import example.hq.spike.data.HqClient.TaskItem
import example.hq.spike.ui.chat.ChatRow
import example.hq.spike.ui.tasks.TasksLogic

/** What one search looks across: everything, or one kind of thing. */
enum class SearchScope(val label: String) { All("All"), Notes("Notes"), Tasks("Tasks"), Chats("Chats") }

/** The matching done on the phone for tasks and chats: the lists are already here, so it works with HQ down. */
object SearchMatch {
    const val MAX_PER_GROUP = 20
    const val RECENT_KEPT = 8

    private fun words(q: String) = tidy(q).lowercase().split(' ').filter { it.isNotEmpty() }

    /** Every word of the query has to be in the text, in any order. */
    fun hits(text: String, q: String): Boolean {
        val ws = words(q)
        if (ws.isEmpty()) return false
        val t = text.lowercase()
        return ws.all { it in t }
    }

    fun tasks(all: List<TaskItem>, q: String): List<TaskItem> =
        all.filter { hits("${it.displayId} ${it.title} ${it.description}", q) }.sortedWith(TasksLogic.byUrgency).take(MAX_PER_GROUP)

    fun chats(all: List<ChatRow>, q: String): List<ChatRow> = all.filter { hits("${it.title} ${it.subtitle}", q) }.take(MAX_PER_GROUP)

    /** The newest search first, each once, and not the empty one. */
    fun remember(recent: List<String>, q: String): List<String> {
        val t = tidy(q)
        if (t.isEmpty()) return recent
        return (listOf(t) + recent.filter { !it.equals(t, ignoreCase = true) }).distinct().take(RECENT_KEPT)
    }

    /** One line, single spaces: recent searches are stored one per line, so a query with a line break would split into two. */
    fun tidy(q: String): String = q.trim().replace(Regex("\\s+"), " ")

    /** The words of a task's state, as the Tasks list says them. */
    fun statusWords(t: TaskItem): String = TasksLogic.statusWords(t.status)
}
