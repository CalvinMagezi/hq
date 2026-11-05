package example.hq.spike.ui.search

import example.hq.spike.R
import example.hq.spike.ui.shell.Words

/** The words on the Search screen, kept out of the layout so a test can read them. */
object SearchWords {
    const val PREVIEW_ROWS = 5
    const val ASK_MAX = 10

    fun scope(w: Words, s: SearchScope): String = w.s(
        when (s) {
            SearchScope.All -> R.string.search_scope_all
            SearchScope.Notes -> R.string.search_scope_notes
            SearchScope.Tasks -> R.string.search_scope_tasks
            SearchScope.Chats -> R.string.search_scope_chats
            SearchScope.Helpers -> R.string.search_scope_helpers
        },
    )

    fun field(w: Words, s: SearchScope): String = w.s(
        when (s) {
            SearchScope.All -> R.string.search_field_all
            SearchScope.Notes -> R.string.search_field_notes
            SearchScope.Tasks -> R.string.search_field_tasks
            SearchScope.Chats -> R.string.search_field_chats
            SearchScope.Helpers -> R.string.search_field_helpers
        },
    )

    fun hint(w: Words, s: SearchScope): String = w.s(if (s == SearchScope.Notes) R.string.search_hint_notes else R.string.search_hint_all)

    fun none(w: Words, s: SearchScope): String = w.s(
        when (s) {
            SearchScope.All -> R.string.search_none_all
            SearchScope.Notes -> R.string.search_none_notes
            SearchScope.Tasks -> R.string.search_none_tasks
            SearchScope.Chats -> R.string.search_none_chats
            SearchScope.Helpers -> R.string.search_none_helpers
        },
    )

    fun seeAll(w: Words, count: Int) = example.hq.spike.ui.shell.Counts.showAll(w, count)

    /** The chip: its name before anything is typed; once it is, the count, or none for notes and chats that HQ could not search. */
    fun chip(w: Words, ui: SearchUi, s: SearchScope): String {
        if (ui.query.isBlank()) return scope(w, s)
        val unknown = (s == SearchScope.Notes || s == SearchScope.Chats) && (ui.hqDown || ui.notesFail != null)
        if (unknown) return scope(w, s)
        val n = countOf(ui, s)
        val capped = s == SearchScope.Notes && ui.notesCapped
        return w.s(if (capped) R.string.search_chip_more else R.string.search_chip_count, scope(w, s), n)
    }

    fun countOf(ui: SearchUi, scope: SearchScope) = when (scope) {
        SearchScope.All -> ui.notes.size + ui.tasks.size + ui.chats.size + ui.helpers.size
        SearchScope.Notes -> ui.notes.size
        SearchScope.Tasks -> ui.tasks.size
        SearchScope.Chats -> ui.chats.size
        SearchScope.Helpers -> ui.helpers.size
    }

    /** True when there is nothing to show and nothing still on its way; a failed ask of notes is never "nothing matches". */
    fun noMatch(ui: SearchUi): Boolean {
        if (ui.query.isBlank() || ui.loading) return false
        val failed = ui.notesFail != null
        return when (ui.scope) {
            SearchScope.All -> countOf(ui, SearchScope.All) == 0 && !failed
            SearchScope.Notes -> ui.notes.isEmpty() && !failed
            SearchScope.Tasks -> ui.tasks.isEmpty()
            SearchScope.Chats -> ui.chats.isEmpty() && !failed
            SearchScope.Helpers -> ui.helpers.isEmpty()
        }
    }

    /** The Ask HQ pill: how many notes it takes (at most [ASK_MAX]) and what it says; null when there are none to take. */
    fun ask(w: Words, notes: Int): String? = when {
        notes <= 0 -> null
        notes == 1 -> w.s(R.string.search_ask_one)
        notes > ASK_MAX -> w.s(R.string.search_ask_first, ASK_MAX)
        else -> w.s(R.string.search_ask_many, notes)
    }

    /** The folder chips: "All folders N" then each folder, or nothing when HQ cut the list and the counts would be wrong. */
    fun folderChips(ui: SearchUi): List<Pair<String?, Int>> {
        val f = ui.folders ?: return emptyList()
        if (ui.scope != SearchScope.Notes || f.isEmpty()) return emptyList()
        return listOf<Pair<String?, Int>>(null to f.values.sum()) + f.entries.sortedWith(compareByDescending<Map.Entry<String, Int>> { it.value }.thenBy { it.key != "Inbox" }.thenBy { it.key }).map { it.key to it.value }
    }

    /** "Projects" or "Projects / Agent-HQ": the folder as the owner reads it. */
    fun trail(folder: String): String = folder.split('/').filter { it.isNotBlank() }.joinToString(" / ")

    fun savedLine(w: Words, ui: SearchUi): String? {
        val at = ui.savedAtMs ?: return null
        return w.s(R.string.search_saved_tasks, w.clockOrDay(java.time.Instant.ofEpochMilli(at), java.time.Instant.now()))
    }

    fun helperLine(w: Words, ui: SearchUi, h: example.hq.spike.ui.helpers.HelperCard): String {
        val base = w.s(R.string.search_helper_row, h.title, h.where, example.hq.spike.ui.helpers.HelperWords.stateWords(w, h.state))
        if (!ui.server) return base
        val at = ui.savedAtMs?.let { w.s(R.string.search_helper_saved, w.clockOrDay(java.time.Instant.ofEpochMilli(it), java.time.Instant.now())) }
        return listOfNotNull(base, w.s(R.string.search_helper_server), at).joinToString(" · ")
    }
}
