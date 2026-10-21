package example.hq.spike.ui.tasks

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.data.HqClient.TaskComment
import example.hq.spike.data.HqClient.TaskItem
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqUnavailable
import java.time.LocalDate
import java.util.UUID
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.json.JSONObject

data class TaskLine(val text: String, val action: String? = null)

/** A task opened on its own page, with what hangs off it. [prompt] is "send back", "status" or "priority" while that step is open. */
data class TaskDetail(
    val task: TaskItem,
    val subtasks: List<TaskItem> = emptyList(),
    val comments: List<TaskComment> = emptyList(),
    val prompt: String? = null,
    val draft: String = "",
    val error: String? = null,
    val loading: Boolean = true,
)

/** The quick-add sheet: one line, or the longer form after "Add details". */
data class QuickState(
    val text: String = "",
    val more: Boolean = false,
    val notes: String = "",
    val priority: String? = null,
    val due: LocalDate? = null,
    val saving: Boolean = false,
    val error: String? = null,
)

data class TasksUi(
    val loaded: Boolean = false,
    val offline: Boolean = false,
    val tab: TaskTab = TaskTab.Today,
    val tasks: List<TaskItem> = emptyList(),
    val expanded: Set<String> = emptySet(),
    val hidden: Set<String> = emptySet(),
    val detail: TaskDetail? = null,
    val quick: QuickState? = null,
    val line: TaskLine? = null,
    val today: LocalDate = LocalDate.now(),
) {
    val shown: List<TaskItem> get() = tasks.filter { it.id !in hidden }
    val groups: List<TaskGroup> get() = when (tab) {
        TaskTab.Today -> TasksLogic.today(shown, today)
        TaskTab.Upcoming -> TasksLogic.upcoming(shown, today)
        TaskTab.All -> TasksLogic.all(shown)
    }
}

class TasksActions(
    val onTab: (TaskTab) -> Unit = {},
    val onRefresh: () -> Unit = {},
    val onToggleGroup: (String) -> Unit = {},
    val onDone: (TaskItem) -> Unit = {},
    val onTomorrow: (TaskItem) -> Unit = {},
    val onUndo: () -> Unit = {},
    val onDismissLine: () -> Unit = {},
    val onOpen: (TaskItem) -> Unit = {},
    val onClose: () -> Unit = {},
    val onMarkDone: () -> Unit = {},
    val onPrompt: (String?) -> Unit = {},
    val onDraft: (String) -> Unit = {},
    val onSendBack: () -> Unit = {},
    val onStatus: (String) -> Unit = {},
    val onPriority: (String?) -> Unit = {},
    val onToggleSub: (TaskItem) -> Unit = {},
    val onAddSub: () -> Unit = {},
    val onComment: () -> Unit = {},
    val onArchive: () -> Unit = {},
    val onQuick: (QuickState?) -> Unit = {},
    val onQuickSave: () -> Unit = {},
)

class TasksViewModel(app: Application) : AndroidViewModel(app) {
    private val client = HqServices.get(app).client
    private val _state = MutableStateFlow(TasksUi())
    val state: StateFlow<TasksUi> = _state
    private var frozen = false
    private var undo: (suspend () -> Unit)? = null
    private var lineJob: Job? = null

    val actions = TasksActions(
        onTab = { t -> _state.update { it.copy(tab = t) } }, onRefresh = { viewModelScope.launch { refresh() } },
        onToggleGroup = { k -> _state.update { it.copy(expanded = if (k in it.expanded) it.expanded - k else it.expanded + k) } },
        onDone = ::done, onTomorrow = ::tomorrow, onUndo = ::undoLast, onDismissLine = { _state.update { it.copy(line = null) } },
        onOpen = ::open, onClose = { _state.update { it.copy(detail = null) } }, onMarkDone = { _state.value.detail?.task?.let(::done); _state.update { it.copy(detail = null) } },
        onPrompt = { p -> detail { it.copy(prompt = p, draft = "") } }, onDraft = { d -> detail { it.copy(draft = d) } }, onSendBack = ::sendBack,
        onStatus = { s -> patchDetail(JSONObject().put("status", s)) }, onPriority = { p -> patchDetail(JSONObject().put("priority", p ?: JSONObject.NULL)) },
        onToggleSub = ::toggleSub, onAddSub = ::addSub, onComment = ::comment, onArchive = ::archive,
        onQuick = { q -> _state.update { it.copy(quick = q) } }, onQuickSave = ::saveQuick,
    )

    init {
        viewModelScope.launch {
            while (true) {
                if (!frozen) refresh()
                delay(POLL_MS)
            }
        }
    }

    fun freeze(fixed: TasksUi) {
        frozen = true
        _state.value = fixed
    }

    suspend fun refresh() {
        val tasks = try { client.tasks() } catch (e: HqUnavailable) { null }
        if (frozen) return
        _state.update { if (tasks == null) it.copy(loaded = true, offline = true) else it.copy(loaded = true, offline = false, tasks = tasks, hidden = emptySet(), today = LocalDate.now()) }
    }

    private fun detail(change: (TaskDetail) -> TaskDetail) = _state.update { s -> s.detail?.let { s.copy(detail = change(it)) } ?: s }

    private fun say(text: String, action: String? = null, undoWith: (suspend () -> Unit)? = null) {
        undo = undoWith
        lineJob?.cancel()
        _state.update { it.copy(line = TaskLine(text, action)) }
        lineJob = viewModelScope.launch { delay(LINE_MS); _state.update { it.copy(line = null) }; undo = null }
    }

    /** Finishes a task at once and offers Undo; a 409 says it changed meanwhile and reloads. */
    private fun done(t: TaskItem) {
        _state.update { it.copy(hidden = it.hidden + t.id) }
        viewModelScope.launch {
            try {
                client.patchTask(t.id, JSONObject().put("status", "complete").put("expected_status", t.status))
                say("Done: ${t.title.take(TITLE_IN_LINE)}", "Undo") {
                    client.patchTask(t.id, JSONObject().put("status", t.status).put("expected_status", "complete"))
                    refresh()
                }
            } catch (e: HqUnavailable) {
                _state.update { it.copy(hidden = it.hidden - t.id) }
                say(if (e.message?.contains("HTTP 409") == true) "Changed meanwhile. Updated." else "Could not finish it. HQ is not answering.")
                refresh()
            }
        }
    }

    /** Moves a task to tomorrow by setting its start date; refused when it is due before then. */
    private fun tomorrow(t: TaskItem) {
        val date = LocalDate.now().plusDays(1)
        _state.update { it.copy(hidden = it.hidden + t.id) }
        viewModelScope.launch {
            try {
                client.patchTask(t.id, JSONObject().put("start_date", date.toString()))
                say("Moved to tomorrow", "Undo") {
                    client.patchTask(t.id, JSONObject().put("start_date", t.startDate ?: JSONObject.NULL))
                    refresh()
                }
                refresh()
            } catch (e: HqUnavailable) {
                _state.update { it.copy(hidden = it.hidden - t.id) }
                say(if (e.message?.contains("HTTP 400") == true) "It is due before tomorrow, so it cannot move." else "Could not move it. HQ is not answering.")
            }
        }
    }

    private fun undoLast() {
        val action = undo ?: return
        undo = null
        lineJob?.cancel()
        _state.update { it.copy(line = null) }
        viewModelScope.launch { try { action() } catch (e: HqUnavailable) { say("Could not undo that. HQ is not answering.") } }
    }

    /**
     * Opens the task with this short id over whatever is showing, so Back returns there. If the list does not have it
     * (not loaded yet, a sub-task, offline) it is looked for once more, and [onMissing] is called if it is still not there.
     */
    fun openRef(ref: String, onMissing: () -> Unit) {
        viewModelScope.launch {
            val task = _state.value.tasks.firstOrNull { it.displayId == ref } ?: run { refresh(); _state.value.tasks.firstOrNull { it.displayId == ref } }
            if (task != null) open(task) else { say("Could not find $ref in your tasks."); onMissing() }
        }
    }

    private fun open(t: TaskItem) {
        _state.update { it.copy(detail = TaskDetail(t)) }
        viewModelScope.launch {
            val subs = try { client.tasks(parentId = t.id) } catch (e: HqUnavailable) { emptyList() }
            val comments = try { client.taskComments(t.id) } catch (e: HqUnavailable) { emptyList() }
            detail { if (it.task.id == t.id) it.copy(subtasks = subs, comments = comments, loading = false) else it }
        }
    }

    private fun patchDetail(change: JSONObject) {
        val d = _state.value.detail ?: return
        detail { it.copy(prompt = null) }
        viewModelScope.launch {
            try {
                val updated = client.patchTask(d.task.id, change.put("expected_status", d.task.status))
                detail { it.copy(task = updated) }
                refresh()
            } catch (e: HqUnavailable) {
                detail { it.copy(error = if (e.message?.contains("HTTP 409") == true) "Changed meanwhile." else "Could not save. HQ is not answering.") }
            }
        }
    }

    /** "Send back": the answer to "What should change?" goes in as a comment, then the task returns to In progress. */
    private fun sendBack() {
        val d = _state.value.detail ?: return
        if (d.draft.isBlank()) return
        viewModelScope.launch {
            try {
                client.addTaskComment(d.task.id, d.draft.trim())
                client.patchTask(d.task.id, JSONObject().put("status", "in_progress").put("expected_status", "ready_for_review"))
                _state.update { it.copy(detail = null) }
                say("Sent back to hq")
                refresh()
            } catch (e: HqUnavailable) {
                detail { it.copy(error = "Could not send it back. HQ is not answering.") }
            }
        }
    }

    private fun toggleSub(s: TaskItem) {
        val to = if (s.status == "complete") "to_do" else "complete"
        detail { d -> d.copy(subtasks = d.subtasks.map { if (it.id == s.id) it.copy(status = to) else it }) }
        viewModelScope.launch {
            try { client.patchTask(s.id, JSONObject().put("status", to).put("expected_status", s.status)); refresh() } catch (e: HqUnavailable) { open(_state.value.detail?.task ?: return@launch) }
        }
    }

    private fun addSub() {
        val d = _state.value.detail ?: return
        if (d.draft.isBlank() || d.prompt != "sub") return
        viewModelScope.launch {
            try {
                client.createTask(JSONObject().put("title", d.draft.trim()).put("parent_task_id", d.task.id).put("created_by", "operator"))
                open(d.task)
                refresh()
            } catch (e: HqUnavailable) { detail { it.copy(error = "Could not add it. HQ is not answering.") } }
        }
    }

    private fun comment() {
        val d = _state.value.detail ?: return
        if (d.draft.isBlank() || d.prompt != null) return
        viewModelScope.launch {
            try {
                client.addTaskComment(d.task.id, d.draft.trim())
                detail { it.copy(draft = "", comments = emptyList()) }
                val comments = client.taskComments(d.task.id)
                detail { it.copy(comments = comments) }
            } catch (e: HqUnavailable) { detail { it.copy(error = "Could not post the comment. HQ is not answering.") } }
        }
    }

    private fun archive() {
        val t = _state.value.detail?.task ?: return
        _state.update { it.copy(detail = null, hidden = it.hidden + t.id) }
        viewModelScope.launch {
            try {
                client.archiveTask(t.id)
                say("Task archived", "Undo") { client.restoreTask(t.id); refresh() }
            } catch (e: HqUnavailable) {
                _state.update { it.copy(hidden = it.hidden - t.id) }
                say("Could not archive it. HQ is not answering.")
            }
        }
    }

    private fun saveQuick() {
        val q = _state.value.quick ?: return
        val parsed = QuickAdd.parse(q.text, LocalDate.now())
        if (parsed.title.isBlank() || q.saving) return
        _state.update { it.copy(quick = q.copy(saving = true, error = null)) }
        viewModelScope.launch {
            try {
                val body = JSONObject().put("title", parsed.title).put("created_by", "operator").put("external_id", "phone-" + UUID.randomUUID())
                (q.priority ?: parsed.priority)?.let { body.put("priority", it) }
                parsed.startDate?.let { body.put("start_date", it.toString()) }
                q.due?.let { body.put("due_date", it.toString()) }
                if (q.notes.isNotBlank()) body.put("description", q.notes.trim())
                parsed.initiative?.let { body.put("initiative", it.replaceFirstChar { c -> c.uppercase() }) }
                client.createTask(body)
                _state.update { it.copy(quick = null) }
                say("Task added")
                refresh()
            } catch (e: HqUnavailable) {
                _state.update { it.copy(quick = it.quick?.copy(saving = false, error = "Could not add it. HQ is not answering.")) }
            }
        }
    }

    private companion object {
        const val POLL_MS = 30_000L
        const val LINE_MS = 5_000L
        const val TITLE_IN_LINE = 40
    }
}
