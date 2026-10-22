package example.hq.spike.ui.tasks

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.data.HqClient.TaskComment
import example.hq.spike.data.HqClient
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
    /** What was typed, per place it was typed in (the comment box, send back, a new sub-task), so opening one does not erase another. */
    val drafts: Map<String, String> = emptyMap(),
    val error: String? = null,
    val loading: Boolean = true,
    /** A write from this page is on its way; another tap waits for it. */
    val busy: Boolean = false,
    /** Goes with a new sub-task, so sending it again after a lost answer cannot add it twice. */
    val subKey: String = newKey(),
) {
    val draft: String get() = drafts[prompt ?: COMMENT_DRAFT] ?: ""

    /** The draft kept for [place] once [sent] has gone out, unless more was typed meanwhile. */
    fun withoutSent(place: String, sent: String): Map<String, String> = if (drafts[place]?.trim() == sent) drafts - place else drafts
}

private const val COMMENT_DRAFT = ""
private fun newKey() = "phone-" + UUID.randomUUID()

/** The quick-add sheet: one line, or the longer form after "Add details". */
data class QuickState(
    val text: String = "",
    val more: Boolean = false,
    val notes: String = "",
    val priority: String? = null,
    val due: LocalDate? = null,
    val saving: Boolean = false,
    val error: String? = null,
    /** Goes with the new task, so sending it again after a lost answer cannot add it twice. */
    val key: String = newKey(),
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
    /** Which list the All tab shows, the archived tasks (read only when that list is chosen), and the helpers working now. */
    val chip: AllChip = AllChip.Active,
    val archived: List<TaskItem> = emptyList(),
    val sessions: List<HqClient.WorkSession> = emptyList(),
) {
    val workingIds: Set<String> get() = sessions.mapTo(HashSet()) { it.taskId }
    val shown: List<TaskItem> get() = tasks.filter { it.id !in hidden }
    val groups: List<TaskGroup> get() = when (tab) {
        TaskTab.Today -> TasksLogic.today(shown, today)
        TaskTab.Upcoming -> TasksLogic.upcoming(shown, today)
        TaskTab.All -> TasksLogic.allList(shown, chip, archived)
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
    val onChip: (AllChip) -> Unit = {},
    val onRestore: () -> Unit = {},
)

class TasksViewModel(app: Application) : AndroidViewModel(app) {
    private val client = HqServices.get(app).client
    private val _state = MutableStateFlow(TasksUi())
    val state: StateFlow<TasksUi> = _state
    private var frozen = false
    private var undo: (suspend () -> Unit)? = null
    private var lineJob: Job? = null
    private var refreshSeq = 0
    private val writing = mutableSetOf<String>()

    val actions = TasksActions(
        onTab = { t -> _state.update { it.copy(tab = t) } }, onRefresh = { viewModelScope.launch { refresh() } },
        onToggleGroup = { k -> _state.update { it.copy(expanded = if (k in it.expanded) it.expanded - k else it.expanded + k) } },
        onDone = ::done, onTomorrow = ::tomorrow, onUndo = ::undoLast, onDismissLine = { _state.update { it.copy(line = null) } },
        onOpen = ::open, onClose = { _state.update { it.copy(detail = null) } }, onMarkDone = { _state.value.detail?.task?.let(::done); _state.update { it.copy(detail = null) } },
        onPrompt = { p -> detail { it.copy(prompt = p) } },
        onDraft = { d -> detail { it.copy(drafts = it.drafts + ((it.prompt ?: COMMENT_DRAFT) to d)) } }, onSendBack = ::sendBack,
        onStatus = { s -> patchDetail(JSONObject().put("status", s)) }, onPriority = { p -> patchDetail(JSONObject().put("priority", p ?: JSONObject.NULL)) },
        onToggleSub = ::toggleSub, onAddSub = ::addSub, onComment = ::comment, onArchive = ::archive,
        onQuick = { q -> _state.update { it.copy(quick = q) } }, onQuickSave = ::saveQuick,
        onChip = ::chooseChip, onRestore = ::restore,
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

    /** A refresh that began before a write finished is dropped, so it cannot bring a finished task back. */
    suspend fun refresh() {
        val seq = ++refreshSeq
        val tasks = try { client.tasks() } catch (e: HqUnavailable) { null }
        // The extras are only read for the list that shows them, and a failed read keeps what was there.
        val onAll = _state.value.tab == TaskTab.All
        val sessions = if (onAll && tasks != null) try { client.liveWorkSessions() } catch (e: HqUnavailable) { null } else null
        val archived = if (onAll && tasks != null && _state.value.chip == AllChip.Archived) try { client.tasks(archived = true) } catch (e: HqUnavailable) { null } else null
        if (frozen || seq != refreshSeq) return
        _state.update { s ->
            if (tasks == null) s.copy(loaded = true, offline = true)
            else s.copy(
                loaded = true, offline = false, tasks = tasks, sessions = sessions ?: s.sessions, archived = archived ?: s.archived, hidden = s.hidden.filter { it in writing }.toSet(), today = LocalDate.now(),
                detail = s.detail?.let { d -> tasks.firstOrNull { it.id == d.task.id }?.let { fresh -> d.copy(task = fresh) } ?: d },
            )
        }
    }

    private fun chooseChip(chip: AllChip) {
        _state.update { it.copy(chip = chip) }
        if (chip == AllChip.Archived) viewModelScope.launch { refresh() }
    }

    /** Brings an archived task back, with the sub-tasks archived along with it. */
    private fun restore() {
        val t = _state.value.detail?.task ?: return
        viewModelScope.launch {
            try {
                client.restoreTask(t.id)
                _state.update { it.copy(detail = null, archived = it.archived.filter { a -> a.id != t.id }) }
                say("Task restored")
                refresh()
            } catch (e: HqUnavailable) {
                say(why(e, "Could not restore it. HQ is not answering."))
            }
        }
    }

    private fun detail(change: (TaskDetail) -> TaskDetail) = _state.update { s -> s.detail?.let { s.copy(detail = change(it)) } ?: s }

    /** Changes the open page only if it is still the page of task [id]; an answer for a task left behind is ignored. */
    private fun detailOf(id: String, change: (TaskDetail) -> TaskDetail) = _state.update { s -> if (s.detail?.task?.id == id) s.copy(detail = change(s.detail)) else s }

    private fun begin(id: String) {
        writing += id
        _state.update { it.copy(hidden = it.hidden + id) }
    }

    private fun succeeded(id: String) {
        writing -= id
        refreshSeq++
    }

    private fun failed(id: String) {
        writing -= id
        _state.update { it.copy(hidden = it.hidden - id) }
    }

    private fun why(e: HqUnavailable, notAnswering: String) = when (e.status) {
        HTTP_CONFLICT -> "Changed meanwhile. Updated."
        HTTP_BAD_REQUEST -> "HQ would not accept that."
        HTTP_NOT_FOUND -> "That task is not there any more."
        else -> notAnswering
    }

    private fun say(text: String, action: String? = null, undoWith: (suspend () -> Unit)? = null) {
        undo = undoWith
        lineJob?.cancel()
        _state.update { it.copy(line = TaskLine(text, action)) }
        lineJob = viewModelScope.launch { delay(LINE_MS); _state.update { it.copy(line = null) }; undo = null }
    }

    /** Finishes a task at once and offers Undo; a 409 says it changed meanwhile and reloads. */
    private fun done(t: TaskItem) {
        begin(t.id)
        viewModelScope.launch {
            try {
                client.patchTask(t.id, JSONObject().put("status", "complete").put("expected_status", t.status))
                succeeded(t.id)
                say("Done: ${t.title.take(TITLE_IN_LINE)}", "Undo") {
                    val back = JSONObject().put("status", t.status).put("expected_status", "complete")
                    // Leaving "blocked" clears the reason, so it goes back with the status.
                    if (t.status == "blocked") t.blockedReason?.let { back.put("blocked_reason", it) }
                    client.patchTask(t.id, back)
                    refresh()
                }
            } catch (e: HqUnavailable) {
                failed(t.id)
                say(why(e, "Could not finish it. HQ is not answering."))
                refresh()
            }
        }
    }

    /** Moves a task to tomorrow by setting its start date; refused when it is due before then. */
    private fun tomorrow(t: TaskItem) {
        val date = LocalDate.now().plusDays(1)
        begin(t.id)
        viewModelScope.launch {
            try {
                client.patchTask(t.id, JSONObject().put("start_date", date.toString()))
                succeeded(t.id)
                say("Moved to tomorrow", "Undo") {
                    client.patchTask(t.id, JSONObject().put("start_date", t.startDate ?: JSONObject.NULL))
                    refresh()
                }
                refresh()
            } catch (e: HqUnavailable) {
                failed(t.id)
                say(if (e.status == HTTP_BAD_REQUEST) "It is due before tomorrow, so it cannot move." else why(e, "Could not move it. HQ is not answering."))
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

    /** Opens the task with this id, as [openRef] does for a short id. */
    fun openById(id: String, onMissing: () -> Unit) {
        viewModelScope.launch {
            val task = _state.value.tasks.firstOrNull { it.id == id } ?: run { refresh(); _state.value.tasks.firstOrNull { it.id == id } }
            if (task != null) open(task) else { say("Could not find that task."); onMissing() }
        }
    }

    private fun open(t: TaskItem) {
        _state.update { it.copy(detail = TaskDetail(t)) }
        if (t.archivedAt != null) { detailOf(t.id) { it.copy(loading = false) }; return }
        viewModelScope.launch {
            val subs = try { client.tasks(parentId = t.id) } catch (e: HqUnavailable) { emptyList() }
            val comments = try { client.taskComments(t.id) } catch (e: HqUnavailable) { emptyList() }
            detailOf(t.id) { it.copy(subtasks = subs, comments = comments, loading = false) }
        }
    }

    private fun patchDetail(change: JSONObject) {
        val d = _state.value.detail ?: return
        if (d.busy) return
        val id = d.task.id
        detailOf(id) { it.copy(prompt = null, busy = true, error = null) }
        viewModelScope.launch {
            try {
                val updated = client.patchTask(id, change.put("expected_status", d.task.status))
                detailOf(id) { it.copy(task = updated, busy = false) }
                refresh()
            } catch (e: HqUnavailable) {
                detailOf(id) { it.copy(busy = false, error = why(e, "Could not save. HQ is not answering.")) }
                if (e.status == HTTP_CONFLICT) refresh()
            }
        }
    }

    /** "Send back": the answer to "What should change?" goes in as a comment, then the task returns to In progress. */
    private fun sendBack() {
        val d = _state.value.detail ?: return
        if (d.draft.isBlank() || d.busy) return
        val id = d.task.id
        val sent = d.draft.trim()
        detailOf(id) { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                client.addTaskComment(id, sent)
            } catch (e: HqUnavailable) {
                detailOf(id) { it.copy(busy = false, error = "Could not send it back. HQ is not answering.") }
                return@launch
            }
            // The note is in, so it is not typed again if the move below fails.
            detailOf(id) { it.copy(drafts = it.withoutSent("sendback", sent)) }
            try {
                client.patchTask(id, JSONObject().put("status", "in_progress").put("expected_status", "ready_for_review"))
                closeIfOpen(id)
                say("Sent back to hq")
            } catch (e: HqUnavailable) {
                if (e.status == HTTP_CONFLICT) {
                    closeIfOpen(id)
                    say("Changed meanwhile. Your note was added.")
                } else {
                    detailOf(id) { it.copy(busy = false, prompt = null, error = "Your note was added, but the task did not move. Use the status button to move it.") }
                }
            }
            refresh()
        }
    }

    private fun closeIfOpen(id: String) = _state.update { s -> if (s.detail?.task?.id == id) s.copy(detail = null) else s }

    private fun toggleSub(s: TaskItem) {
        val parent = _state.value.detail?.task?.id ?: return
        val to = if (s.status == "complete") "to_do" else "complete"
        detailOf(parent) { d -> d.copy(subtasks = d.subtasks.map { if (it.id == s.id) it.copy(status = to) else it }) }
        viewModelScope.launch {
            try {
                client.patchTask(s.id, JSONObject().put("status", to).put("expected_status", s.status))
                refresh()
            } catch (e: HqUnavailable) {
                val subs = try { client.tasks(parentId = parent) } catch (e2: HqUnavailable) { null }
                detailOf(parent) { d -> d.copy(subtasks = subs ?: d.subtasks, error = why(e, "Could not save. HQ is not answering.")) }
            }
        }
    }

    private fun addSub() {
        val d = _state.value.detail ?: return
        if (d.draft.isBlank() || d.prompt != "sub" || d.busy) return
        val id = d.task.id
        val sent = d.draft.trim()
        detailOf(id) { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                // The key follows the words, so sending the same ones again after a lost answer adds nothing twice, and different ones are new.
                client.createTask(JSONObject().put("title", sent).put("parent_task_id", id).put("created_by", "operator").put("external_id", "${d.subKey}-${sent.hashCode()}"))
            } catch (e: HqUnavailable) {
                detailOf(id) { it.copy(busy = false, error = why(e, "Could not add it. HQ is not answering.")) }
                return@launch
            }
            val subs = try { client.tasks(parentId = id) } catch (e: HqUnavailable) { null }
            detailOf(id) { it.copy(prompt = null, drafts = it.withoutSent("sub", sent), subtasks = subs ?: it.subtasks, busy = false, subKey = newKey()) }
            refresh()
        }
    }

    private fun comment() {
        val d = _state.value.detail ?: return
        if (d.draft.isBlank() || d.prompt != null || d.busy) return
        val id = d.task.id
        val sent = d.draft.trim()
        detailOf(id) { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                client.addTaskComment(id, sent)
            } catch (e: HqUnavailable) {
                detailOf(id) { it.copy(busy = false, error = why(e, "Could not post the comment. HQ is not answering.")) }
                return@launch
            }
            val comments = try { client.taskComments(id) } catch (e: HqUnavailable) { null }
            detailOf(id) { it.copy(drafts = it.withoutSent(COMMENT_DRAFT, sent), comments = comments ?: it.comments, busy = false) }
        }
    }

    private fun archive() {
        val t = _state.value.detail?.task ?: return
        _state.update { it.copy(detail = null) }
        begin(t.id)
        viewModelScope.launch {
            try {
                client.archiveTask(t.id)
                succeeded(t.id)
                say("Task archived", "Undo") { client.restoreTask(t.id); refresh() }
            } catch (e: HqUnavailable) {
                failed(t.id)
                say(if (e.status == HTTP_BAD_REQUEST && t.subtaskCount > 0) "Archive its sub-tasks first." else why(e, "Could not archive it. HQ is not answering."))
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
                val body = JSONObject().put("title", parsed.title).put("created_by", "operator").put("external_id", "${q.key}-${listOf(q.text, q.notes, q.priority, q.due).hashCode()}")
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
                _state.update { it.copy(quick = it.quick?.copy(saving = false, error = why(e, "Could not add it. HQ is not answering."))) }
            }
        }
    }

    private companion object {
        const val POLL_MS = 30_000L
        const val LINE_MS = 5_000L
        const val TITLE_IN_LINE = 40
        const val HTTP_BAD_REQUEST = 400
        const val HTTP_NOT_FOUND = 404
        const val HTTP_CONFLICT = 409
    }
}
