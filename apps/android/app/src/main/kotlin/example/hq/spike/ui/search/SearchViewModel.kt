package example.hq.spike.ui.search

import android.app.Application
import android.content.Context
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.R
import example.hq.spike.data.Connection
import example.hq.spike.data.HqClient.TaskItem
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqUnavailable
import example.hq.spike.ui.chat.ChatRow
import example.hq.spike.ui.chat.blocks.NoteRefs
import example.hq.spike.ui.helpers.HelperCard
import example.hq.spike.ui.notes.SearchHit
import example.hq.spike.ui.startup.StartRules
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** Why the notes could not be searched, one per failure line of wave-4/02-search.md section 5. */
enum class NotesFail(val line: Int) {
    Starting(R.string.fail_search_notes_chats_starting),
    Restarting(R.string.fail_search_notes_chats_restarting),
    Stopped(R.string.fail_search_notes_chats_stopped),
    Timeout(R.string.fail_search_notes_chats_timeout),
    ;

    companion object {
        /** The line for the state HQ is in when the ask failed; any other state is a plain no-answer. */
        fun of(c: Connection): NotesFail = when (c) {
            is Connection.Starting -> Starting
            is Connection.Restarting -> Restarting
            is Connection.Stopped -> Stopped
            else -> Timeout
        }
    }
}

data class SearchUi(
    val open: Boolean = false,
    val query: String = "",
    val scope: SearchScope = SearchScope.All,
    /** A notes folder (under Notebooks) the search stays in, asked of HQ, never filtered on the phone. */
    val folder: String? = null,
    val notes: List<SearchHit> = emptyList(),
    val tasks: List<TaskItem> = emptyList(),
    val chats: List<ChatRow> = emptyList(),
    val helpers: List<HelperCard> = emptyList(),
    val loading: Boolean = false,
    /** Set when notes (and so chats) could not be searched; null otherwise, so "Nothing matches that" is never said for a failed ask. */
    val notesFail: NotesFail? = null,
    /** HQ is not answering, so chats are not searched and their chip has no number. */
    val hqDown: Boolean = false,
    /** Notes per folder, from the hits of a search with no folder; null once HQ returned a full page, as the count would lie. */
    val folders: Map<String, Int>? = null,
    val recent: List<String> = emptyList(),
    /** Recent searches were just cleared and Undo is offered. */
    val cleared: Boolean = false,
    /** When the copy of tasks and helpers shown while HQ is down was saved, in epoch milliseconds. */
    val savedAtMs: Long? = null,
    /** The helpers are on a server HQ, which the row says. */
    val server: Boolean = false,
) {
    val notesCapped: Boolean get() = notes.size >= SearchViewModel.NOTE_HITS
}

class SearchActions(
    val onOpen: (SearchSources, SearchStart) -> Unit = { _, _ -> },
    val onClose: () -> Unit = {},
    /** Hides the search to show a result, keeping it so Back can bring it back. */
    val onHide: () -> Unit = {},
    val onQuery: (String) -> Unit = {},
    val onScope: (SearchScope) -> Unit = {},
    val onFolder: (String?) -> Unit = {},
    val onRecent: (String) -> Unit = {},
    val onPicked: (String) -> Unit = {},
    val onClearRecent: () -> Unit = {},
    val onUndoClear: () -> Unit = {},
    val onClearedShown: () -> Unit = {},
    val onScrolled: (Int, Int) -> Unit = { _, _ -> },
)

/** The lists the phone already has, matched here so tasks and helpers still search while HQ is down. */
data class SearchSources(val tasks: List<TaskItem> = emptyList(), val chats: List<ChatRow> = emptyList(), val helpers: List<HelperCard> = emptyList(), val server: Boolean = false)

/** One field across notes, tasks, chats and helpers. Notes ask HQ (and so do chats' rows); tasks and helpers are matched here. */
class SearchViewModel(app: Application) : AndroidViewModel(app) {
    private val services = HqServices.get(app)
    private val client = services.client
    private val prefs = app.getSharedPreferences("search", Context.MODE_PRIVATE)
    private val _state = MutableStateFlow(SearchUi())
    val state: StateFlow<SearchUi> = _state
    private var sources = SearchSources()
    private var job: Job? = null
    private var cleared: List<String> = emptyList()

    /** The search as it was when a result was opened, so Back from that page can bring it back (rule 9). */
    private var parked: SearchUi? = null
    var scrollAt: Pair<Int, Int> = 0 to 0
        private set

    val actions = SearchActions(
        onOpen = ::open, onClose = ::close, onHide = { _state.update { it.copy(open = false) } }, onQuery = ::query, onScope = ::scope, onFolder = ::folder, onRecent = ::query,
        onPicked = ::remember, onClearRecent = ::clearRecent, onUndoClear = ::undoClear,
        onClearedShown = { _state.update { it.copy(cleared = false) } },
        onScrolled = { i, o -> scrollAt = i to o },
    )

    init {
        // When HQ answers again, a search that could not read notes runs itself again.
        viewModelScope.launch {
            services.status.collect { c ->
                val down = down(c)
                val s = _state.value
                if (s.open && s.hqDown != down) {
                    _state.update { it.copy(hqDown = down) }
                    if (!down && s.query.isNotBlank()) query(s.query)
                }
            }
        }
    }

    private fun down(c: Connection) = c is Connection.Starting || c is Connection.Restarting || c is Connection.Stopped

    /** Opens the one search where a link, a pill or a magnifier asks to start; the lists to match come with it. */
    private fun open(from: SearchSources, start: SearchStart) {
        sources = from
        parked = null
        job?.cancel()
        scrollAt = 0 to 0
        _state.update { SearchUi(open = true, scope = start.scope, folder = start.folder, recent = recent(), hqDown = down(services.status.value), server = from.server) }
        if (start.query.isNotBlank()) query(start.query)
    }

    /** Back from a result: the same query, scope, folder and list, once. Returns false when there is nothing to bring back. */
    fun resume(): Boolean {
        val back = parked ?: return false
        parked = null
        _state.value = back.copy(open = true, recent = recent())
        return true
    }

    val canResume: Boolean get() = parked != null

    private fun close() {
        job?.cancel()
        parked = null
        _state.update { SearchUi(recent = it.recent) }
    }

    private fun recent(): List<String> = prefs.getString(RECENT, "").orEmpty().split('\n').filter { it.isNotBlank() }.distinct()

    /** A result was chosen: the words that found it are kept, and the search waits to come back. */
    private fun remember(q: String) {
        val kept = SearchMatch.remember(recent(), q)
        prefs.edit().putString(RECENT, kept.joinToString("\n")).apply()
        _state.update { it.copy(recent = kept) }
        parked = _state.value
    }

    private fun clearRecent() {
        cleared = recent()
        prefs.edit().putString(RECENT, SearchMatch.clearRecent().joinToString("\n")).apply()
        _state.update { it.copy(recent = emptyList(), cleared = true) }
    }

    private fun undoClear() {
        prefs.edit().putString(RECENT, cleared.joinToString("\n")).apply()
        _state.update { it.copy(recent = cleared, cleared = false) }
    }

    private fun scope(s: SearchScope) = _state.update { it.copy(scope = s) }

    private fun folder(f: String?) {
        _state.update { it.copy(folder = f) }
        query(_state.value.query)
    }

    private fun query(q: String) {
        job?.cancel()
        val down = down(services.status.value)
        // Matched before the state is updated: a long task list is not walked inside the update, which can run again.
        val tasks = SearchMatch.tasks(sources.tasks, q)
        val helpers = SearchMatch.helpers(sources.helpers, q)
        val chats = if (down) emptyList() else SearchMatch.chats(sources.chats, q)
        val searching = q.isNotBlank()
        _state.update {
            it.copy(
                query = q, tasks = tasks, chats = chats, helpers = helpers, hqDown = down, cleared = false,
                notes = if (searching) it.notes else emptyList(), folders = if (searching) it.folders else null,
                notesFail = if (searching && down) NotesFail.of(services.status.value) else null,
                loading = searching && !down,
                savedAtMs = StartRules.lastAnswer(getApplication()).takeIf { t -> down && t > 0 },
            )
        }
        if (!searching || down) return
        val folder = _state.value.folder
        job = viewModelScope.launch {
            delay(DEBOUNCE_MS)
            val found = try {
                client.searchNoteHits(q.trim(), NOTE_HITS, folder).map { SearchHit(it.path, it.title.ifBlank { NoteRefs.title(it.path) }, it.notebook, it.snippet) }
            } catch (e: HqUnavailable) { null }
            _state.update { s ->
                if (s.query != q || s.folder != folder) s
                else s.copy(
                    notes = found ?: emptyList(), loading = false, notesFail = if (found == null) NotesFail.of(services.status.value) else null,
                    folders = if (found != null && folder == null) folderCounts(found) else s.folders,
                )
            }
        }
    }

    /** Notes per top folder from a whole page of hits, or null when HQ cut the list and a count would be wrong. */
    private fun folderCounts(hits: List<SearchHit>): Map<String, Int>? {
        if (hits.size >= NOTE_HITS) return null
        return hits.groupingBy { SearchMatch.folderOf(it.path) }.eachCount().filterKeys { it.isNotBlank() }
    }

    companion object {
        private const val RECENT = "recent"
        private const val DEBOUNCE_MS = 250L
        const val NOTE_HITS = 30
    }
}
