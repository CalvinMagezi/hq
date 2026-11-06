package example.hq.spike.ui.notes

import android.app.Application
import android.content.Context
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqUnavailable
import example.hq.spike.data.NoteChanged
import example.hq.spike.data.NoteNotFound
import example.hq.spike.ui.chat.NoteExporter
import example.hq.spike.ui.chat.Notice
import example.hq.spike.ui.chat.NoteSheet
import example.hq.spike.ui.chat.blocks.NoteRefs
import example.hq.spike.ui.chat.blocks.NoteText
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.json.JSONObject

/** Everything the Notes pages can ask for. */
class NotesActions(
    val onRefresh: () -> Unit = {},
    val onOpenFolder: (String) -> Unit = {},
    val onOpenNote: (String) -> Unit = {},
    val onBack: () -> Unit = {},
    val onNew: (String) -> Unit = {},
    val onEdit: () -> Unit = {},
    val onMenu: (Boolean) -> Unit = {},
    val onTogglePin: () -> Unit = {},
    val onMakeTask: () -> Unit = {},
    val onExport: () -> Unit = {},
    val onExportFormat: (example.hq.spike.ui.chat.ExportFormat) -> Unit = {},
    val onExportClose: () -> Unit = {},
    val onExportAgain: () -> Unit = {},
    val onNotice: (String) -> Unit = {},
    val onDismissNotice: () -> Unit = {},
    val onTitle: (String) -> Unit = {},
    val onText: (TextFieldValue) -> Unit = {},
    val onPreview: () -> Unit = {},
    val onChooseFolder: (Boolean) -> Unit = {},
    val onPickFolder: (String) -> Unit = {},
    val onSave: () -> Unit = {},
    val onSeeTheirs: () -> Unit = {},
    val onKeepBoth: () -> Unit = {},
    val onLeave: () -> Unit = {},
    val onKeepEditing: () -> Unit = {},
    val onDiscard: () -> Unit = {},
    /** The note page being left reports how far it was scrolled (first visible item, offset), so Back can return to the same place. */
    val onScrolled: (String, Int, Int) -> Unit = { _, _, _ -> },
    val scrollOf: (String) -> Pair<Int, Int>? = { null },
)

class NotesViewModel(app: Application) : AndroidViewModel(app) {
    private val services = HqServices.get(app)
    private val client = services.client
    private val drafts = app.getSharedPreferences("note_drafts", Context.MODE_PRIVATE)
    private val _state = MutableStateFlow(NotesUi())
    val state: StateFlow<NotesUi> = _state
    private val exporter = NoteExporter(app, client, viewModelScope)
    private var frozen = false

    private val scrolls = ScrollMemory()

    val actions = NotesActions(
        onRefresh = ::refresh, onOpenFolder = ::openFolder, onOpenNote = ::openNote,
        onBack = ::back, onNew = ::newNote, onEdit = ::edit, onMenu = { open -> _state.update { it.copy(menu = open) } },
        onTogglePin = ::togglePin, onMakeTask = ::makeTask,
        onExport = { (_state.value.reader as? ReaderPhase.Ready)?.note?.let { n -> _state.update { it.copy(menu = false) }; exporter.choose(n.path, n.title, n.body) } },
        onExportFormat = exporter::start, onExportClose = exporter::close, onExportAgain = exporter::again,
        onNotice = { text -> _state.update { it.copy(notice = Notice(text)) } }, onDismissNotice = { _state.update { it.copy(notice = null) } },
        onTitle = { t -> edit { w -> w.copy(title = t, status = SaveStatus.Unsaved) } },
        onText = { t -> edit { w -> w.copy(text = t, status = if (t.text != w.original) SaveStatus.Unsaved else w.status).also { keepDraft(it) } } },
        onPreview = { edit { it.copy(preview = !it.preview) } },
        onChooseFolder = { open -> edit { it.copy(chooser = open) } },
        onPickFolder = { f -> edit { it.copy(folder = f, chooser = false) } },
        onSave = ::save, onSeeTheirs = ::seeTheirs, onKeepBoth = ::keepBoth, onLeave = ::leave, onKeepEditing = { edit { it.copy(leaving = false) } }, onDiscard = ::discard,
        onScrolled = scrolls::save, scrollOf = scrolls::restore,
    )


    init {
        viewModelScope.launch { exporter.state.collect { e -> _state.update { it.copy(export = e) } } }
    }

    /** Draws a fixed state for checking the pages against the design with no hq. */
    fun freeze(fixed: NotesUi) {
        frozen = true
        _state.value = fixed
    }

    private fun push(route: NotesRoute) = _state.update { it.copy(stack = it.stack + route, menu = false) }

    private fun back() {
        val w = _state.value.write
        if (_state.value.top is NotesRoute.Write && w != null && w.dirty && !w.leaving) { edit { it.copy(leaving = true) }; return }
        popNow()
    }

    private fun popNow() {
        _state.update { s -> s.copy(stack = s.stack.dropLast(1), menu = false, write = if (s.top is NotesRoute.Write) null else s.write) }
        // The folder and the note each have one slot, so the page now showing is loaded again rather than left as the one just closed.
        when (val top = _state.value.top) {
            is NotesRoute.Folder -> if (_state.value.folder?.path != top.path) loadFolder(top.path)
            is NotesRoute.Reader -> if ((_state.value.reader as? ReaderPhase.Ready)?.note?.path != top.path) loadReader(top.path)
            else -> {}
        }
        if (_state.value.stack.isEmpty()) refresh()
    }

    /** Closes every page, as when a result is sent to a chat. */
    fun closeAll() = _state.update { it.copy(stack = emptyList(), write = null, menu = false) }

    /** True when a page is open, so the system Back key should pop it. */
    fun hasPage(): Boolean = _state.value.stack.isNotEmpty()

    fun refresh() {
        if (frozen) return
        viewModelScope.launch {
            val recent = try { client.recentNotes() } catch (e: HqUnavailable) { null }
            val pinned = try { client.pinnedNotes() } catch (e: HqUnavailable) { null }
            val folders = try { client.folders() } catch (e: HqUnavailable) { null }
            _state.update {
                if (recent == null && pinned == null && folders == null) it.copy(loaded = true, offline = true)
                else it.copy(loaded = true, offline = false, recent = recent ?: it.recent, pinned = pinned ?: it.pinned, folders = folders ?: it.folders)
            }
        }
    }

    private fun openFolder(path: String) {
        push(NotesRoute.Folder(path))
        loadFolder(path)
    }

    private fun loadFolder(path: String) {
        _state.update { it.copy(folder = FolderView(path)) }
        viewModelScope.launch {
            val listing = try { client.folderListing(path) } catch (e: HqUnavailable) { null }
            _state.update { s ->
                if ((s.folder?.path) != path) s
                else s.copy(folder = FolderView(path, listing?.folders.orEmpty(), listing?.notes.orEmpty(), loading = false), offline = s.offline || listing == null)
            }
        }
    }

    /** Opens a note on its own page; a link inside a note pushes another. */
    fun openNote(path: String) {
        if (_state.value.top == NotesRoute.Reader(path)) return
        scrolls.forget(path)
        push(NotesRoute.Reader(path))
        loadReader(path)
    }

    /** Opens the note a `hq://notes/n/{id}` link names, wherever it lives now; an unknown id shows the gone page. */
    fun openNoteById(id: String) {
        viewModelScope.launch {
            val phase = try {
                openNote(client.notePathById(id))
                return@launch
            } catch (e: NoteNotFound) {
                ReaderPhase.Gone
            } catch (e: HqUnavailable) {
                ReaderPhase.Failed
            }
            push(NotesRoute.Reader(UNKNOWN_NOTE))
            _state.update { if (it.top == NotesRoute.Reader(UNKNOWN_NOTE)) it.copy(reader = phase) else it }
        }
    }

    private fun loadReader(path: String) {
        // A link by id that named no note has no path to ask HQ for.
        if (path == UNKNOWN_NOTE) {
            _state.update { if (it.top == NotesRoute.Reader(path)) it.copy(reader = ReaderPhase.Gone) else it }
            return
        }
        _state.update { it.copy(reader = ReaderPhase.Loading) }
        viewModelScope.launch {
            val phase = try {
                val body = client.note(path)
                val pins = try { client.pinned() } catch (e: HqUnavailable) { emptySet() }
                val sheet = NoteSheet(body.path, NoteRefs.title(body.path), NoteRefs.folderTrail(body.path), body.content, body.truncated, null, body.path in pins, body.hash)
                val edited = (_state.value.recent + _state.value.pinned).firstOrNull { it.path == body.path }?.let { NoteTime.edited(it.mtime) }.orEmpty()
                ReaderPhase.Ready(sheet, edited)
            } catch (e: NoteNotFound) {
                ReaderPhase.Gone
            } catch (e: HqUnavailable) {
                ReaderPhase.Failed
            }
            _state.update { if (it.top == NotesRoute.Reader(path)) it.copy(reader = phase) else it }
        }
    }


    private fun togglePin() {
        val ready = _state.value.reader as? ReaderPhase.Ready ?: return
        val pin = !ready.note.pinned
        _state.update { it.copy(menu = false) }
        viewModelScope.launch {
            try {
                client.setPinned(ready.note.path, pin)
                _state.update { s -> s.copy(reader = ready.copy(note = ready.note.copy(pinned = pin)), notice = Notice(getApplication<Application>().getString(if (pin) example.hq.spike.R.string.notes_pinned else example.hq.spike.R.string.notes_unpinned))) }
            } catch (e: HqUnavailable) {
                _state.update { it.copy(notice = Notice(getApplication<Application>().getString(example.hq.spike.R.string.notes_pin_no_reply))) }
            }
        }
    }

    /** A task in the Inbox that points at this note. The same note twice answers with the first task, so a second tap adds nothing. */
    private fun makeTask() {
        val note = (_state.value.reader as? ReaderPhase.Ready)?.note ?: return
        viewModelScope.launch {
            val said = try {
                getApplication<Application>().getString(if (client.createTaskOnce(noteTaskBody(note.path, note.title))) example.hq.spike.R.string.notes_has_task else example.hq.spike.R.string.notes_task_added)
            } catch (e: HqUnavailable) {
                getApplication<Application>().getString(if (e.status == 0) example.hq.spike.R.string.notes_task_no_reply else example.hq.spike.R.string.notes_task_failed)
            }
            _state.update { it.copy(notice = Notice(said)) }
        }
    }

    private fun newNote(folder: String) {
        val draft = drafts.getString(draftKey(null, folder), null)
        val text = draft ?: ""
        _state.update { it.copy(write = WriteState(null, folder, "", TextFieldValue(text, TextRange(text.length)), original = "", status = SaveStatus.Saved)) }
        push(NotesRoute.Write(null))
    }

    private fun edit() {
        val ready = _state.value.reader as? ReaderPhase.Ready ?: return
        if (ready.note.truncated) {
            _state.update { it.copy(menu = false, notice = Notice(getApplication<Application>().getString(example.hq.spike.R.string.notes_too_long_edit))) }
            return
        }
        val body = NoteText.withoutFirstBlock(ready.note.body)
        val draft = drafts.getString(draftKey(ready.note.path, ""), null)
        val text = draft ?: body
        _state.update {
            it.copy(write = WriteState(ready.note.path, NoteRefs.folderTrail(ready.note.path), ready.note.title, TextFieldValue(text, TextRange(text.length)), original = body, status = if (text != body) SaveStatus.Unsaved else SaveStatus.Saved, base = ready.note.hash))
        }
        push(NotesRoute.Write(ready.note.path))
    }

    private fun edit(change: (WriteState) -> WriteState) = _state.update { s -> s.write?.let { s.copy(write = change(it)) } ?: s }

    private fun keepDraft(w: WriteState) {
        drafts.edit().putString(draftKey(w.path, w.folder), w.text.text).apply()
    }

    private fun save() {
        val w = _state.value.write ?: return
        if (w.status == SaveStatus.Saving) return
        if (w.isNew && w.title.isBlank() && w.text.text.isBlank()) {
            _state.update { it.copy(notice = Notice(getApplication<Application>().getString(example.hq.spike.R.string.notes_write_first))) }
            return
        }
        edit { it.copy(status = SaveStatus.Saving, leaving = false) }
        viewModelScope.launch {
            try {
                val path = w.path ?: client.createNote(w.folder, w.title.ifBlank { firstLine(w.text.text) }, w.text.text)
                if (w.path != null) client.saveNote(path, w.text.text, w.base)
                drafts.edit().remove(draftKey(w.path, w.folder)).apply()
                _state.update { it.copy(write = null, stack = it.stack.dropLast(1)) }
                if (w.path == null) openNote(path) else reloadReader(path)
            } catch (e: NoteChanged) {
                edit { it.copy(status = SaveStatus.Unsaved, conflict = Conflict(e.by)) }
            } catch (e: HqUnavailable) {
                edit { it.copy(status = SaveStatus.Failed) }
            }
        }
    }

    /** Shows what the note says now, beside the owner's own text, without losing either. */
    private fun seeTheirs() {
        val w = _state.value.write ?: return
        val path = w.path ?: return
        val c = w.conflict ?: return
        edit { it.copy(conflict = c.copy(busy = true)) }
        viewModelScope.launch {
            val theirs = try { NoteText.withoutFirstBlock(client.note(path).content) } catch (e: java.io.IOException) { null }
            edit { it.copy(conflict = it.conflict?.copy(theirs = theirs ?: "", busy = false)) }
        }
    }

    /** Saves the owner's text as a new note beside the original, then shows the note as it is now. */
    private fun keepBoth() {
        val w = _state.value.write ?: return
        val path = w.path ?: return
        val c = w.conflict ?: return
        edit { it.copy(conflict = c.copy(busy = true)) }
        viewModelScope.launch {
            try {
                val title = getApplication<Application>().getString(example.hq.spike.R.string.notes_your_version, w.title)
                client.createNote(path.substringBeforeLast('/'), title, w.text.text)
                drafts.edit().remove(draftKey(w.path, w.folder)).apply()
                _state.update { it.copy(write = null, stack = it.stack.dropLast(1)) }
                reloadReader(path)
            } catch (e: HqUnavailable) {
                edit { it.copy(status = SaveStatus.Failed, conflict = it.conflict?.copy(busy = false)) }
            }
        }
    }

    private fun reloadReader(path: String) {
        val pinned = (_state.value.reader as? ReaderPhase.Ready)?.note?.pinned ?: false
        _state.update { it.copy(reader = ReaderPhase.Loading) }
        viewModelScope.launch {
            val phase = try {
                val body = client.note(path)
                ReaderPhase.Ready(NoteSheet(body.path, NoteRefs.title(body.path), NoteRefs.folderTrail(body.path), body.content, body.truncated, null, pinned, body.hash), "edited just now")
            } catch (e: NoteNotFound) { ReaderPhase.Gone } catch (e: HqUnavailable) { ReaderPhase.Failed }
            _state.update { if (it.top == NotesRoute.Reader(path)) it.copy(reader = phase) else it }
        }
    }

    private fun leave() = back()

    private fun discard() {
        _state.value.write?.let { drafts.edit().remove(draftKey(it.path, it.folder)).apply() }
        popNow()
    }

    private fun firstLine(text: String) = text.lineSequence().firstOrNull { it.isNotBlank() }?.trim('#', ' ')?.take(TITLE_MAX).orEmpty()

    private fun draftKey(path: String?, folder: String) = path ?: "new:$folder"

    private companion object {
        const val SEARCH_WAIT_MS = 250L
        const val TITLE_MAX = 60
    }
}
