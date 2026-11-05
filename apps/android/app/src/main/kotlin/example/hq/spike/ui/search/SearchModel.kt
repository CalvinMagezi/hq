package example.hq.spike.ui.search

import example.hq.spike.data.HqClient.TaskItem
import example.hq.spike.ui.chat.ChatRow
import example.hq.spike.ui.helpers.HelperCard
import example.hq.spike.ui.tasks.TasksLogic

/** What one search looks across: everything, or one kind of thing. */
enum class SearchScope { All, Notes, Tasks, Chats, Helpers }

/** Where a search starts: the scope, a notes folder to stay in (under Notebooks, no leading slash) and the words already typed. */
data class SearchStart(val scope: SearchScope = SearchScope.All, val folder: String? = null, val query: String = "")

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

    /** Helpers are matched on the phone by what they work on and who does it. */
    fun helpers(all: List<HelperCard>, q: String): List<HelperCard> =
        all.filter { hits("${it.title} ${it.agent} ${it.where} ${it.line} ${it.folder}", q) }.take(MAX_PER_GROUP)

    /** Nothing left to remember. */
    fun clearRecent(): List<String> = emptyList()

    /** The newest search first, each once, and not the empty one. */
    fun remember(recent: List<String>, q: String): List<String> {
        val t = tidy(q)
        if (t.isEmpty()) return recent
        return (listOf(t) + recent.filter { !it.equals(t, ignoreCase = true) }).distinct().take(RECENT_KEPT)
    }

    /** The top folder of a note under Notebooks, "Projects" for Notebooks/Projects/Agent-HQ/x.md; blank for a note at the root. */
    fun folderOf(path: String): String = path.removePrefix("Notebooks/").substringBefore('/', "")

    /** One line, single spaces: recent searches are stored one per line, so a query with a line break would split into two. */
    fun tidy(q: String): String = q.trim().replace(Regex("\\s+"), " ")

    /** The words of a task's state, as the Tasks list says them. */
    fun statusWords(w: example.hq.spike.ui.shell.Words, t: TaskItem): String = TasksLogic.statusWords(w, t.status)
}
