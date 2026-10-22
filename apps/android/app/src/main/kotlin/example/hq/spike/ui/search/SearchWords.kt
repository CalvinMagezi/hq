package example.hq.spike.ui.search

/** The words on the Search screen, kept out of the layout so a test can read them. */
object SearchWords {
    const val FIELD = "Search notes, tasks and chats"
    const val PREVIEW_ROWS = 5
    const val RECENT_TITLE = "Recent searches"
    const val EMPTY_HINT = "Type to search notes, tasks and chats."
    const val NO_MATCH = "Nothing matches that."
    const val NOTES_DOWN = "Could not search notes. HQ is not answering."

    fun seeAll(count: Int) = "See all $count"

    /** The chip: its name, and how many it found once something is typed. */
    fun chip(scope: SearchScope, found: Int, searching: Boolean) = if (searching) "${scope.label} $found" else scope.label

    fun countOf(ui: SearchUi, scope: SearchScope) = when (scope) {
        SearchScope.All -> ui.notes.size + ui.tasks.size + ui.chats.size
        SearchScope.Notes -> ui.notes.size
        SearchScope.Tasks -> ui.tasks.size
        SearchScope.Chats -> ui.chats.size
    }

    /** True when there is nothing to show and nothing still on its way. */
    fun noMatch(ui: SearchUi): Boolean {
        if (ui.query.isBlank() || ui.loading) return false
        return when (ui.scope) {
            SearchScope.All -> countOf(ui, SearchScope.All) == 0 && !ui.notesFailed
            SearchScope.Notes -> ui.notes.isEmpty() && !ui.notesFailed
            SearchScope.Tasks -> ui.tasks.isEmpty()
            SearchScope.Chats -> ui.chats.isEmpty()
        }
    }
}
