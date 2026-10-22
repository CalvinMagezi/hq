package example.hq.spike.ui.search

import android.app.Application
import android.content.Context
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.data.HqClient.TaskItem
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqUnavailable
import example.hq.spike.ui.chat.ChatRow
import example.hq.spike.ui.notes.SearchHit
import example.hq.spike.ui.chat.blocks.NoteRefs
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class SearchUi(
    val open: Boolean = false,
    val query: String = "",
    val scope: SearchScope = SearchScope.All,
    val notes: List<SearchHit> = emptyList(),
    val tasks: List<TaskItem> = emptyList(),
    val chats: List<ChatRow> = emptyList(),
    val loading: Boolean = false,
    val notesFailed: Boolean = false,
    val recent: List<String> = emptyList(),
)

class SearchActions(
    val onOpen: (List<TaskItem>, List<ChatRow>) -> Unit = { _, _ -> },
    val onClose: () -> Unit = {},
    val onQuery: (String) -> Unit = {},
    val onScope: (SearchScope) -> Unit = {},
    val onRecent: (String) -> Unit = {},
    val onPicked: (String) -> Unit = {},
)

/** One field across notes, tasks and chats. Notes ask HQ; tasks and chats are matched here, from the lists the app already has. */
class SearchViewModel(app: Application) : AndroidViewModel(app) {
    private val client = HqServices.get(app).client
    private val prefs = app.getSharedPreferences("search", Context.MODE_PRIVATE)
    private val _state = MutableStateFlow(SearchUi())
    val state: StateFlow<SearchUi> = _state
    private var allTasks: List<TaskItem> = emptyList()
    private var allChats: List<ChatRow> = emptyList()
    private var job: Job? = null

    val actions = SearchActions(
        onOpen = ::open, onClose = { job?.cancel(); _state.update { it.copy(open = false, query = "", notes = emptyList(), tasks = emptyList(), chats = emptyList(), loading = false) } },
        onQuery = ::query, onScope = { s -> _state.update { it.copy(scope = s) } }, onRecent = ::query,
        onPicked = { q -> remember(q) },
    )

    private fun open(tasks: List<TaskItem>, chats: List<ChatRow>) {
        allTasks = tasks
        allChats = chats
        _state.update { SearchUi(open = true, recent = recent()) }
    }

    private fun recent(): List<String> = prefs.getString(RECENT, "").orEmpty().split('\n').filter { it.isNotBlank() }.distinct()

    /** A result was chosen, so the words that found it are worth keeping. */
    private fun remember(q: String) {
        val kept = SearchMatch.remember(recent(), q)
        prefs.edit().putString(RECENT, kept.joinToString("\n")).apply()
        _state.update { it.copy(recent = kept) }
    }

    private fun query(q: String) {
        job?.cancel()
        // Matched before the state is updated: a long task list is not walked inside the update, which can run again.
        val tasks = SearchMatch.tasks(allTasks, q)
        val chats = SearchMatch.chats(allChats, q)
        _state.update { it.copy(query = q, tasks = tasks, chats = chats, notes = if (q.isBlank()) emptyList() else it.notes, notesFailed = false, loading = q.isNotBlank()) }
        if (q.isBlank()) return
        job = viewModelScope.launch {
            delay(DEBOUNCE_MS)
            val found = try {
                client.searchNoteHits(q.trim(), NOTE_HITS).map { SearchHit(it.path, it.title.ifBlank { NoteRefs.title(it.path) }, it.notebook, it.snippet) }
            } catch (e: HqUnavailable) { null }
            _state.update { s -> if (s.query != q) s else s.copy(notes = found ?: emptyList(), notesFailed = found == null, loading = false) }
        }
    }

    private companion object {
        const val RECENT = "recent"
        const val DEBOUNCE_MS = 250L
        const val NOTE_HITS = 30
    }
}
