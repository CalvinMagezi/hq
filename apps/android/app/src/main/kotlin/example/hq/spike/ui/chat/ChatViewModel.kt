package example.hq.spike.ui.chat

import android.app.Application
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.core.content.FileProvider
import java.io.File
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.data.ChatEvent
import example.hq.spike.data.HqClient
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqStatus
import example.hq.spike.data.HqUnavailable
import example.hq.spike.data.NoteNotFound
import example.hq.spike.data.StoredMessage
import example.hq.spike.ui.chat.blocks.Attachment
import example.hq.spike.ui.chat.blocks.Attachments
import example.hq.spike.ui.chat.blocks.Block
import example.hq.spike.ui.chat.blocks.ChartSpec
import example.hq.spike.ui.chat.blocks.NoteRefs
import example.hq.spike.ui.chat.blocks.StepCards
import java.util.UUID
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** Everything the chat screen can ask for. A plain bag of functions, so the screen can also be drawn from a fixed state. */
class ChatActions(
    val onDraft: (String) -> Unit = {},
    val onSend: () -> Unit = {},
    val onStop: () -> Unit = {},
    val onRetry: () -> Unit = {},
    val onOpenChats: () -> Unit = {},
    val onCloseChats: () -> Unit = {},
    val onNewChat: () -> Unit = {},
    val onOpenChat: (String) -> Unit = {},
    val onSearchChats: (String) -> Unit = {},
    val onArchive: (String) -> Unit = {},
    val onUndo: () -> Unit = {},
    val onDismissNotice: () -> Unit = {},
    val onNotice: (String) -> Unit = {},
    val onOpenNote: (String) -> Unit = {},
    val onCloseNote: () -> Unit = {},
    val onAskAboutNote: (String) -> Unit = {},
    val onOpenChart: (ChartSpec) -> Unit = {},
    val onOpenTable: (Block.Table) -> Unit = {},
    val onClosePage: () -> Unit = {},
    val onOpenFile: (Attachment) -> Unit = {},
    val onRemoveContext: (ContextRef) -> Unit = {},
    val onOpenAdd: () -> Unit = {},
    val onCloseAdd: () -> Unit = {},
    val onPickKind: (ContextRef.Kind) -> Unit = {},
    val onPickerQuery: (String) -> Unit = {},
    val onClosePicker: () -> Unit = {},
    val onChoose: (ContextRef) -> Unit = {},
    val onFiles: (List<Uri>) -> Unit = {},
    val onRetryFile: (String) -> Unit = {},
    val onRemoveFile: (String) -> Unit = {},
    val onLongPress: (String) -> Unit = {},
    val onCloseSheet: () -> Unit = {},
    val onSelectText: (String) -> Unit = {},
    val onRegenerate: (String) -> Unit = {},
    val onEditResend: (String) -> Unit = {},
    val onCancelEdit: () -> Unit = {},
    val onLoadOlder: () -> Unit = {},
    val onSlash: (String) -> Unit = {},
    val onNoteMenu: (Boolean) -> Unit = {},
    val onTogglePin: () -> Unit = {},
    val onExport: () -> Unit = {},
    val onExportFormat: (ExportFormat) -> Unit = {},
    val onExportClose: () -> Unit = {},
    val onExportAgain: () -> Unit = {},
)

class ChatViewModel(app: Application) : AndroidViewModel(app) {
    private val services = HqServices.get(app)
    private val client = services.client
    private val drafts = app.getSharedPreferences(DRAFT_PREFS, Context.MODE_PRIVATE)
    private val _state = MutableStateFlow(ChatUiState())
    val state: StateFlow<ChatUiState> = _state

    /** Client ids of the questions this phone sent, so their saved copies can replace the local ones. */
    private val pending = mutableSetOf<String>()

    /** What was attached to each question in flight, so a send that fails puts the chips back. */
    private val sentContexts = mutableMapOf<String, List<ContextRef>>()
    private val sentFiles = mutableMapOf<String, List<PendingFile>>()
    private val composer = ComposerController(app, client, _state, viewModelScope)
    private val exporter = NoteExporter(app, client, _state, viewModelScope)
    private var loadingOlder = false
    private class LastQuestion(val threadId: String?, val text: String, val files: List<Attachment>, val contexts: List<ContextRef>)
    private var lastQuestion: LastQuestion? = null
    private val sentReplace = mutableMapOf<String, String>()
    private var archiving: Pair<String, Job>? = null
    private var frozen = false

    val actions = ChatActions(
        onDraft = ::setDraft, onSend = ::send, onStop = ::stop, onRetry = ::retry,
        onOpenChats = ::openChats, onCloseChats = { _state.update { it.copy(chatsOpen = false) } }, onNewChat = { newChat() },
        onOpenChat = ::openThread, onSearchChats = { q -> _state.update { it.copy(chatSearch = q) } },
        onArchive = ::archive, onUndo = ::undoArchive, onDismissNotice = { _state.update { it.copy(notice = null) } },
        onNotice = { text -> _state.update { it.copy(notice = Notice(text)) } },
        onOpenNote = ::openNote, onCloseNote = { _state.update { it.copy(note = null) } }, onAskAboutNote = ::askAbout,
        onOpenChart = { spec -> _state.update { it.copy(page = Page.Chart(spec)) } }, onOpenTable = ::openTable,
        onClosePage = { _state.update { it.copy(page = null) } }, onOpenFile = ::openFile,
        onRemoveContext = composer::removeContext,
        onOpenAdd = composer::openAdd, onCloseAdd = composer::closeAdd, onPickKind = composer::openPicker, onPickerQuery = composer::queryPicker,
        onClosePicker = composer::closePicker, onChoose = composer::choose, onFiles = composer::addFiles, onRetryFile = composer::retry,
        onRemoveFile = composer::removeFile, onLongPress = { id -> _state.update { it.copy(pressed = id) } },
        onCloseSheet = { _state.update { it.copy(pressed = null) } }, onSelectText = { text -> _state.update { it.copy(pressed = null, page = Page.SelectText(text)) } },
        onRegenerate = ::regenerate, onEditResend = ::editResend, onCancelEdit = { _state.update { s -> s.copy(editing = null, draft = "", files = s.files.filter { it.uri.isNotEmpty() }) } },
        onNoteMenu = { open -> _state.update { it.copy(noteMenu = open) } }, onTogglePin = ::togglePin,
        onExport = { _state.value.note?.let(exporter::choose) }, onExportFormat = exporter::start, onExportClose = exporter::close, onExportAgain = exporter::again,
        onLoadOlder = ::loadOlder, onSlash = { name -> setDraft(if (name == "/topic") "/topic " else name) },
    )

    init {
        viewModelScope.launch { services.connection.events.collect { if (!frozen) handle(it) } }
        viewModelScope.launch {
            services.connection.connected.collect { c ->
                if (frozen) return@collect
                _state.update { it.copy(connected = c, offlineSince = if (c) null else it.offlineSince ?: System.currentTimeMillis()) }
                if (c) reconcile()
            }
        }
        // hq may still be starting when the app opens; open the latest chat the first time it answers.
        viewModelScope.launch { services.status.collect { if (it == HqStatus.Live && !frozen) openLatest() } }
    }

    /** Draws a fixed state and stops listening to hq, for checking the screen against the design on a phone with no hq. */
    fun freeze(fixed: ChatUiState) {
        frozen = true
        _state.value = fixed
    }

    private fun handle(event: ChatEvent) {
        _state.update { ChatReducer.apply(it, event, pending) }
        when (event) {
            is ChatEvent.Rejected -> if (event.clientId in pending) restore(event.clientId!!, event.reason)
            is ChatEvent.TurnStart -> event.clientId?.let { sentContexts.remove(it); sentFiles.remove(it); sentReplace.remove(it) }
            is ChatEvent.TurnEnd -> if (event.threadId == _state.value.threadId) viewModelScope.launch {
                // The live copy has tool arguments cut at 4000 characters and no ids; the saved one is whole.
                event.threadId?.let { loadMessages(it) }
                refreshChats()
            }
            is ChatEvent.Lagged -> _state.value.threadId?.let { id -> viewModelScope.launch { loadMessages(id) } }
            else -> {}
        }
    }

    /** After a reconnect: a reply we think is still running may have finished while the socket was down. */
    private suspend fun reconcile() {
        val s = _state.value
        val id = s.threadId ?: return
        val running = try { client.threads().running } catch (e: HqUnavailable) { return }
        if (s.live != null && id !in running) _state.update { it.copy(live = null) }
        loadMessages(id)
    }

    private fun setDraft(text: String) {
        _state.update { it.copy(draft = text) }
        drafts.edit().putString(draftKey(_state.value.threadId), text).apply()
    }

    private fun send() {
        val s = _state.value
        if (s.editing == null && s.files.isEmpty()) SlashCommands.parse(s.draft)?.let { return runSlash(it) }
        submit(s.draft.trim(), replaceFrom = s.editing)
    }

    /** [files] and [contexts] default to what is attached now; a retry or a regenerate passes what the question had. */
    private fun submit(text: String, replaceFrom: String?, files: List<Attachment>? = null, contexts: List<ContextRef>? = null) {
        val s = _state.value
        // A retry or a regenerate re-asks an old question and must leave what is being typed alone.
        val fromComposer = files == null && contexts == null
        val sendFiles = files ?: s.files.mapNotNull { it.uploaded }
        val sendContexts = contexts ?: s.contexts
        if ((text.isBlank() && sendFiles.isEmpty()) || !s.connected || s.running) return
        val clientId = "local-${UUID.randomUUID()}"
        pending += clientId
        lastQuestion = LastQuestion(s.threadId, text, sendFiles, sendContexts)
        val kept = if (replaceFrom == null) s.messages else s.messages.takeWhile { it.id != replaceFrom }
        if (fromComposer) { sentContexts[clientId] = sendContexts; sentFiles[clientId] = s.files }
        if (replaceFrom != null) sentReplace[clientId] = replaceFrom
        _state.update {
            val sent = it.copy(messages = kept + UiMessage(clientId, Speaker.User, text, sending = true, files = sendFiles), failed = false, editing = null)
            if (fromComposer) sent.copy(draft = "", contexts = emptyList(), files = emptyList()) else sent
        }
        if (fromComposer) drafts.edit().remove(draftKey(s.threadId)).apply()
        if (!services.connection.sendChat(withContext(text, sendContexts), s.threadId, clientId, replaceFrom, sendFiles)) restore(clientId, "Not connected to HQ. Your message is back in the box.")
    }

    /** `/clear` and `/topic <name>` archive this chat and open a fresh one, as on the web. */
    private fun runSlash(cmd: SlashCommands.Command) {
        val old = _state.value.threadId
        drafts.edit().remove(draftKey(old)).apply()
        val topic = cmd.args.takeIf { cmd.name == "/topic" }
        if (topic == null) {
            newChat()
            old?.let(::archive)
            return
        }
        viewModelScope.launch {
            try {
                val made = client.createThread(topic)
                _state.update { ChatUiState(connected = it.connected, threadId = made.id, title = made.title.ifBlank { topic }, chats = it.chats) }
                old?.let(::archive)
                refreshChats()
            } catch (e: HqUnavailable) {
                _state.update { it.copy(notice = Notice("Could not start the new chat. HQ is not answering.")) }
            }
        }
    }

    /** Runs the last question again; only the last reply can be replaced, since the server drops what came after. */
    private fun regenerate(replyId: String) {
        val s = _state.value
        val i = s.messages.indexOfFirst { it.id == replyId }
        val question = s.messages.getOrNull(i - 1)?.takeIf { it.speaker == Speaker.User }
        if (question == null || i != s.messages.lastIndex) {
            _state.update { it.copy(pressed = null, notice = Notice("Only the last reply can be run again.")) }
            return
        }
        _state.update { it.copy(pressed = null) }
        submit(question.text, replaceFrom = question.id, files = question.files, contexts = emptyList())
    }

    /** Puts a saved question back in the field; sending it replaces that question and everything after it. */
    private fun editResend(id: String) {
        val m = _state.value.messages.firstOrNull { it.id == id && it.speaker == Speaker.User } ?: return
        val files = m.files.map { PendingFile(UUID.randomUUID().toString(), "", it.name, it.size, it.mime, FileStatus.Ready, uploaded = it) }
        _state.update { it.copy(pressed = null, draft = m.text, editing = m.id, files = files) }
    }

    private fun loadOlder() {
        val s = _state.value
        val tid = s.threadId ?: return
        val oldest = s.messages.firstOrNull { !it.id.startsWith("local-") } ?: return
        if (!s.hasOlder || loadingOlder) return
        loadingOlder = true
        viewModelScope.launch {
            try {
                val page = client.messages(tid, before = oldest.id)
                val older = page.sortedBy { it.createdAt }.map(::toUi)
                _state.update { if (it.threadId == tid) it.copy(messages = older + it.messages, hasOlder = page.size >= HqClient.MESSAGE_PAGE) else it }
            } catch (e: HqUnavailable) {
                // The next time the top is reached it tries again.
            } finally {
                loadingOlder = false
            }
        }
    }

    /** A send that did not go through: the text returns to the field, with the reason. */
    private fun restore(clientId: String, reason: String) {
        pending -= clientId
        val replaced = sentReplace.remove(clientId)
        _state.update { s ->
            val text = s.messages.firstOrNull { it.id == clientId }?.text ?: s.draft
            s.copy(
                messages = s.messages.filter { it.id != clientId }, draft = if (replaced == null) text else s.draft.ifBlank { text }, notice = Notice(reason),
                contexts = sentContexts.remove(clientId) ?: s.contexts, files = sentFiles.remove(clientId) ?: s.files,
                editing = if (replaced != null && s.editing == null) replaced else s.editing,
            )
        }
        // The questions the failed replace cut off locally are still on the server.
        if (replaced != null) _state.value.threadId?.let { id -> viewModelScope.launch { loadMessages(id) } }
    }

    private fun stop() {
        val id = _state.value.threadId ?: return
        if (services.connection.stop(id)) _state.update { it.copy(notice = Notice("Stopped. What I had so far is kept.")) }
    }

    private fun retry() {
        val q = lastQuestion?.takeIf { it.threadId == _state.value.threadId } ?: return
        val last = _state.value.messages.lastOrNull { it.speaker == Speaker.User && !it.sending } ?: return
        submit(q.text, replaceFrom = last.id, files = q.files, contexts = q.contexts)
    }

    private fun newChat(keepSheet: Boolean = false) {
        _state.update {
            ChatUiState(connected = it.connected, draft = drafts.getString(draftKey(null), "").orEmpty(), chats = it.chats, chatsOpen = keepSheet && it.chatsOpen)
        }
    }

    private fun openThread(id: String) {
        val row = _state.value.chats.firstOrNull { it.id == id }
        _state.update {
            it.copy(threadId = id, title = row?.title ?: it.title, messages = emptyList(), live = null, failed = false, chatsOpen = false,
                draft = drafts.getString(draftKey(id), "").orEmpty(), contexts = emptyList(), files = emptyList(), editing = null, pressed = null, hasOlder = false)
        }
        viewModelScope.launch { loadMessages(id) }
    }

    private suspend fun loadMessages(id: String) {
        val page = try { client.messages(id) } catch (e: HqUnavailable) { return }
        val shown = page.sortedBy { it.createdAt }.map(::toUi)
        _state.update {
            if (it.threadId != id) return@update it
            // Sends the server has not acknowledged yet stay at the end.
            val unsaved = it.messages.filter { m -> m.id.startsWith("local-") && shown.none { s -> s.id == m.id } }
            it.copy(messages = shown + unsaved, hasOlder = page.size >= HqClient.MESSAGE_PAGE)
        }
    }

    private fun toUi(m: StoredMessage) = UiMessage.saved(m.id, if (m.role == "user") Speaker.User else Speaker.Reply, m.content, m.tools, m.createdAt)

    /** Fetches the note first and opens the sheet when it is in hand, so a chip can say "Opening…" and a missing note never flashes an empty sheet. */
    private fun openNote(path: String) {
        val s = _state.value
        if (path in s.missingNotes) { _state.update { it.copy(notice = Notice(NOTE_GONE)) }; return }
        if (s.openingNote != null) return
        _state.update { it.copy(openingNote = path) }
        viewModelScope.launch {
            try {
                val body = client.note(path)
                val pinned = try { body.path in client.pinned() } catch (e: HqUnavailable) { false }
                _state.update { st ->
                    val writer = st.messages.lastOrNull { m -> StepCards.of(m.tools).written.any { it.path == body.path } }
                    val age = writer?.let { if (it.createdAt.isBlank()) "just now" else ChatTime.since(it.createdAt) }
                    val sheet = NoteSheet(body.path, NoteRefs.title(body.path), NoteRefs.folderTrail(body.path), body.content, body.truncated, age, pinned)
                    st.copy(openingNote = null, note = sheet)
                }
            } catch (e: NoteNotFound) {
                _state.update { it.copy(openingNote = null, missingNotes = it.missingNotes + path, notice = Notice(NOTE_GONE)) }
            } catch (e: HqUnavailable) {
                _state.update { it.copy(openingNote = null, notice = Notice("Could not open the note. HQ is not answering.")) }
            }
        }
    }

    private fun togglePin() {
        val sheet = _state.value.note ?: return
        val pin = !sheet.pinned
        _state.update { it.copy(noteMenu = false) }
        viewModelScope.launch {
            try {
                client.setPinned(sheet.path, pin)
                _state.update { s -> s.copy(note = s.note?.takeIf { it.path == sheet.path }?.copy(pinned = pin) ?: s.note, notice = Notice(if (pin) "Pinned to the top of Notes" else "Unpinned")) }
            } catch (e: HqUnavailable) {
                _state.update { it.copy(notice = Notice("Could not change the pin. HQ is not answering.")) }
            }
        }
    }

    /** The note joins the next message as a chip; HQ reads only what is attached. */
    private fun askAbout(path: String) {
        val item = ContextRef(ContextRef.Kind.Note, path, NoteRefs.title(path))
        _state.update { it.copy(contexts = if (item in it.contexts) it.contexts else it.contexts + item, note = null) }
    }

    /** Fetches the file to the app's cache and hands it to whichever app opens that kind of file. */
    private fun openFile(file: Attachment) {
        viewModelScope.launch {
            try {
                val dest = File(getApplication<Application>().cacheDir, "chat-files/${file.name.replace('/', '_')}")
                client.download(file.path, dest)
                val app = getApplication<Application>()
                val uri = FileProvider.getUriForFile(app, "${app.packageName}.files", dest)
                val view = Intent(Intent.ACTION_VIEW).setDataAndType(uri, file.mime.ifBlank { "*/*" })
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
                app.startActivity(view)
            } catch (e: HqUnavailable) {
                _state.update { it.copy(notice = Notice("Could not fetch the file. HQ is not answering.")) }
            } catch (e: ActivityNotFoundException) {
                _state.update { it.copy(notice = Notice("No app on this phone can open that file.")) }
            }
        }
    }

    private fun openTable(table: Block.Table) {
        val title = _state.value.title
        val caption = "$title · ${table.rows.size} rows · ${table.headers.size} columns"
        _state.update { it.copy(page = Page.Table(table, caption)) }
    }

    private fun openChats() {
        _state.update { it.copy(chatsOpen = true, chatSearch = "") }
        viewModelScope.launch { refreshChats() }
    }

    private var openedLatest = false

    private suspend fun openLatest() {
        refreshChats()
        val s = _state.value
        val untouched = s.threadId == null && s.messages.isEmpty() && s.draft.isBlank() && !s.chatsOpen
        if (!openedLatest && untouched) s.chats.firstOrNull()?.let { openedLatest = true; openThread(it.id) }
    }

    private suspend fun refreshChats() {
        val list = try { client.threads() } catch (e: HqUnavailable) { return }
        val rows = list.threads.map { t ->
            val group = ChatTime.group(t.updatedAt)
            val (cut, files) = Attachments.split(t.preview.orEmpty())
            val snippet = if (cut.isBlank() && files.isNotEmpty()) "[${files.size} file${if (files.size == 1) "" else "s"}]" else cut.trim()
            val subtitle = if (group == Group.Today && snippet.isNotEmpty()) "${ChatTime.age(t.updatedAt)} · $snippet" else snippet
            ChatRow(t.id, t.title.ifBlank { ChatUiState.NEW_CHAT }, subtitle, group, t.id in list.running, t.unread > 0)
        }
        _state.update { s -> s.copy(chats = rows.filter { it.id != archiving?.first }) }
    }

    /** Archiving waits a few seconds so Undo has something to undo; the server is only told after that. */
    private fun archive(id: String) {
        archiving?.let { (earlier, job) -> job.cancel(); commit(earlier) }
        _state.update { it.copy(chats = it.chats.filter { row -> row.id != id }, notice = Notice("Chat archived", "Undo")) }
        archiving = id to viewModelScope.launch {
            delay(UNDO_MS)
            commit(id)
            archiving = null
        }
    }

    private fun commit(id: String) {
        viewModelScope.launch {
            try { client.archive(id) } catch (e: HqUnavailable) { return@launch }
            if (_state.value.threadId == id) newChat(keepSheet = true)
        }
    }

    private fun undoArchive() {
        archiving?.second?.cancel()
        archiving = null
        _state.update { it.copy(notice = null) }
        viewModelScope.launch { refreshChats() }
    }

    private fun draftKey(threadId: String?) = threadId ?: NEW_CHAT_KEY

    private companion object {
        const val DRAFT_PREFS = "chat_drafts"
        const val NEW_CHAT_KEY = "new"
        const val UNDO_MS = 5_000L
        const val NOTE_GONE = "That note is not in your notes any more"
    }
}
