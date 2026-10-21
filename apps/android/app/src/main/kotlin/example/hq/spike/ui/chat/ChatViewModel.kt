package example.hq.spike.ui.chat

import android.app.Application
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import androidx.core.content.FileProvider
import java.io.File
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.data.ChatEvent
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqStatus
import example.hq.spike.data.HqUnavailable
import example.hq.spike.data.NoteNotFound
import example.hq.spike.data.StoredMessage
import example.hq.spike.ui.chat.blocks.Attachment
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
    val onRemoveContext: (String) -> Unit = {},
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
    private val sentContexts = mutableMapOf<String, List<String>>()
    private var lastQuestion: String? = null
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
        onRemoveContext = { path -> _state.update { it.copy(contexts = it.contexts - path) } },
    )

    init {
        viewModelScope.launch { services.connection.events.collect { if (!frozen) handle(it) } }
        viewModelScope.launch { services.connection.connected.collect { c -> if (!frozen) _state.update { it.copy(connected = c) } } }
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
            is ChatEvent.TurnStart -> event.clientId?.let { sentContexts.remove(it) }
            is ChatEvent.TurnEnd -> if (event.threadId == _state.value.threadId) viewModelScope.launch { refreshChats() }
            is ChatEvent.Lagged -> _state.value.threadId?.let { id -> viewModelScope.launch { loadMessages(id) } }
            else -> {}
        }
    }

    private fun setDraft(text: String) {
        _state.update { it.copy(draft = text) }
        drafts.edit().putString(draftKey(_state.value.threadId), text).apply()
    }

    private fun send() = submit(_state.value.draft.trim(), replaceFrom = null)

    private fun submit(text: String, replaceFrom: String?) {
        val s = _state.value
        if (text.isBlank() || !s.connected || s.running) return
        val clientId = "local-${UUID.randomUUID()}"
        pending += clientId
        lastQuestion = text
        val kept = if (replaceFrom == null) s.messages else s.messages.takeWhile { it.id != replaceFrom }
        sentContexts[clientId] = s.contexts
        _state.update { it.copy(messages = kept + UiMessage(clientId, Speaker.User, text, sending = true), draft = "", failed = false, contexts = emptyList()) }
        drafts.edit().remove(draftKey(s.threadId)).apply()
        if (!services.connection.sendChat(withContext(text, s.contexts), s.threadId, clientId, replaceFrom)) restore(clientId, "Not connected to HQ. Your message is back in the box.")
    }

    /** A send that did not go through: the text returns to the field, with the reason. */
    private fun restore(clientId: String, reason: String) {
        pending -= clientId
        _state.update { s ->
            val text = s.messages.firstOrNull { it.id == clientId }?.text ?: s.draft
            s.copy(messages = s.messages.filter { it.id != clientId }, draft = text, notice = Notice(reason), contexts = sentContexts.remove(clientId) ?: s.contexts)
        }
    }

    private fun stop() {
        val id = _state.value.threadId ?: return
        if (services.connection.stop(id)) _state.update { it.copy(notice = Notice("Stopped. What I had so far is kept.")) }
    }

    private fun retry() {
        val question = lastQuestion ?: return
        val last = _state.value.messages.lastOrNull { it.speaker == Speaker.User && !it.sending } ?: return
        submit(question, replaceFrom = last.id)
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
                draft = drafts.getString(draftKey(id), "").orEmpty())
        }
        viewModelScope.launch { loadMessages(id) }
    }

    private suspend fun loadMessages(id: String) {
        val page = try { client.messages(id) } catch (e: HqUnavailable) { return }
        val shown = page.sortedBy { it.createdAt }.map(::toUi)
        _state.update { if (it.threadId == id && !it.running) it.copy(messages = shown) else it }
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
                _state.update { st ->
                    val writer = st.messages.lastOrNull { m -> StepCards.of(m.tools).written.any { it.path == body.path } }
                    val age = writer?.let { if (it.createdAt.isBlank()) "just now" else ChatTime.since(it.createdAt) }
                    val sheet = NoteSheet(body.path, NoteRefs.title(body.path), NoteRefs.folderTrail(body.path), body.content, body.truncated, age)
                    st.copy(openingNote = null, note = sheet)
                }
            } catch (e: NoteNotFound) {
                _state.update { it.copy(openingNote = null, missingNotes = it.missingNotes + path, notice = Notice(NOTE_GONE)) }
            } catch (e: HqUnavailable) {
                _state.update { it.copy(openingNote = null, notice = Notice("Could not open the note. HQ is not answering.")) }
            }
        }
    }

    /** The note joins the next message as a chip; HQ reads only what is attached. */
    private fun askAbout(path: String) {
        _state.update { it.copy(contexts = if (path in it.contexts) it.contexts else it.contexts + path, note = null) }
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
            val snippet = t.preview.orEmpty()
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
