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
import example.hq.spike.R
import example.hq.spike.data.HqServices
import example.hq.spike.ui.shell.ClientRestore
import example.hq.spike.ui.shell.Undo
import example.hq.spike.ui.shell.UndoKind
import example.hq.spike.data.Connection
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
    /** Answers the offer in the slot above the composer: the reply's id, and Yes, Not now or Later. */
    val onOffer: (String, OfferAnswer) -> Unit = { _, _ -> },
    val onScheduleOffer: (String, Long) -> Unit = { _, _ -> },
    /** "Fewer like this" and "Never show this" on a pill, by HQ's capability id. */
    val onFewer: (String) -> Unit = {},
    val onNever: (String) -> Unit = {},
    /** Pins or unpins a chat in the chats sheet; the pins live on this phone only. */
    val onPinChat: (String) -> Unit = {},
)

class ChatViewModel(app: Application) : AndroidViewModel(app) {
    private val services = HqServices.get(app)
    private val client = services.client
    private val drafts = app.getSharedPreferences(DRAFT_PREFS, Context.MODE_PRIVATE)
    private val pins = app.getSharedPreferences(PIN_PREFS, Context.MODE_PRIVATE)
    private var resultTimer: Job? = null
    private val _state = MutableStateFlow(ChatUiState())
    val state: StateFlow<ChatUiState> = _state

    /** Client ids of the questions this phone sent, so their saved copies can replace the local ones. */
    private val pending = mutableSetOf<String>()

    /** What was attached to each question in flight, so a send that fails puts the chips back. */
    private val sentContexts = mutableMapOf<String, List<ContextRef>>()
    private val sentFiles = mutableMapOf<String, List<PendingFile>>()
    private val composer = ComposerController(app, client, _state, viewModelScope)
    private val exporter = NoteExporter(app, client, viewModelScope)
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
        onArchive = ::archive, onUndo = ::undoArchive, onDismissNotice = { commitArchive(); _state.update { it.copy(notice = null) } },
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
        onExport = { _state.value.note?.let { n -> _state.update { it.copy(noteMenu = false) }; exporter.choose(n.path, n.title, n.body) } }, onExportFormat = exporter::start, onExportClose = exporter::close, onExportAgain = exporter::again,
        onLoadOlder = ::loadOlder, onOffer = ::answerOffer, onScheduleOffer = ::scheduleOffer, onFewer = { id -> tellHq(id, "fewer", R.string.chat_pill_fewer_done) }, onNever = { id -> tellHq(id, "never", R.string.chat_pill_never_done) },
        onPinChat = ::pinChat, onSlash = { name -> setDraft(if (name == "/topic") "/topic " else name) },
    )

    init {
        viewModelScope.launch { exporter.state.collect { e -> _state.update { it.copy(export = e) } } }
        viewModelScope.launch { services.connection.events.collect { if (!frozen) handle(it) } }
        viewModelScope.launch {
            services.connection.connected.collect { c ->
                if (frozen) return@collect
                _state.update { it.copy(connected = c, offlineSince = if (c) null else it.offlineSince ?: System.currentTimeMillis()) }
                if (c) reconcile()
            }
        }
        // hq may still be starting when the app opens; open the latest chat the first time it answers.
        viewModelScope.launch { services.status.collect { if (it == Connection.Connected && !frozen) openLatest() } }
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
            is ChatEvent.TurnStart -> {
                event.clientId?.let { sentContexts.remove(it); sentFiles.remove(it); sentReplace.remove(it) }
                // A first message was sent before its chat had an id; Retry finds it by that id.
                lastQuestion = lastQuestion?.let { q -> if (q.threadId == null && event.threadId != null && event.clientId != null && event.clientId in pending) LastQuestion(event.threadId, q.text, q.files, q.contexts) else q }
            }
            is ChatEvent.TurnEnd -> if (event.threadId == _state.value.threadId) viewModelScope.launch {
                // The live copy has tool arguments cut at 4000 characters and no ids; the saved one is whole.
                event.threadId?.let { loadMessages(it) }
                refreshChats()
            }
            // A turn_end may be among the frames that were lost, so the running state is checked as after a reconnect.
            is ChatEvent.Lagged -> viewModelScope.launch { reconcile() }
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

    /** Text that came from outside (the share sheet): added after what is already typed, never over it. */
    fun addToDraft(text: String) {
        setDraft(draftWith(_state.value.draft, text))
    }

    private fun send() {
        val s = _state.value
        if (s.editing == null && s.files.isEmpty()) SlashCommands.parse(s.draft)?.let { return runSlash(it) }
        // hq swallows exactly this text and never answers, which would leave the message "sending" for good.
        if (s.draft.trim() == "/reset") return _state.update { it.copy(notice = Notice("That command is not used here. Send /clear to start a new chat.")) }
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
            val sent = it.copy(messages = kept + UiMessage(clientId, Speaker.User, text, sending = true, files = sendFiles), failed = false, editing = null, offerResult = null)
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
                _state.update { ChatUiState(connected = it.connected, threadId = made.id, title = made.title.ifBlank { topic }, chats = it.chats, suggestions = it.suggestions, laterSupported = it.laterSupported, pillChoices = it.pillChoices, answeredOffers = it.answeredOffers, pillsCleared = it.pillsCleared) }
                old?.let(::archive)
                refreshChats()
            } catch (e: HqUnavailable) {
                _state.update { it.copy(notice = Notice("Couldn\'t start the new chat. HQ didn\'t answer in time. Try again.")) }
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
            ChatUiState(connected = it.connected, draft = drafts.getString(draftKey(null), "").orEmpty(), chats = it.chats, chatsOpen = keepSheet && it.chatsOpen, suggestions = it.suggestions, laterSupported = it.laterSupported, pillChoices = it.pillChoices, answeredOffers = it.answeredOffers, pillsCleared = it.pillsCleared)
        }
    }

    private fun openThread(id: String) {
        val row = _state.value.chats.firstOrNull { it.id == id }
        _state.update {
            it.copy(threadId = id, title = row?.title ?: it.title, messages = emptyList(), live = null, failed = false, chatsOpen = false,
                draft = drafts.getString(draftKey(id), "").orEmpty(), contexts = emptyList(), files = emptyList(), editing = null, pressed = null, hasOlder = false, offerResult = null)
        }
        viewModelScope.launch { loadMessages(id) }
    }

    /** Opens a chat a link names, but only one that is in the list: an unknown id would blank the open chat and send the next message to nothing. */
    /** The open chat read again (answered offers stay gone, see [OfferFlow.strip]). */
    fun reloadOpen() {
        viewModelScope.launch {
            refreshChats()
            _state.value.threadId?.let { loadMessages(it) }
        }
    }

    /** A Later offer was brought back to [threadId]: open that chat and read it fresh; with no id, read the open one again. */
    fun showBroughtBack(threadId: String?) {
        if (threadId == null) return reloadOpen()
        viewModelScope.launch {
            refreshChats()
            openThread(threadId)
        }
    }

    fun focusComposer() = _state.update { it.copy(focusDraft = it.focusDraft + 1) }

    fun openLinked(id: String) {
        if (_state.value.chats.any { it.id == id }) { openThread(id); return }
        viewModelScope.launch {
            refreshChats()
            if (_state.value.chats.any { it.id == id }) openThread(id) else _state.update { it.copy(notice = Notice(CHAT_GONE)) }
        }
    }

    private suspend fun loadMessages(id: String) {
        val page = try { client.messages(id) } catch (e: HqUnavailable) {
            // HQ says the chat is gone: its pin goes too. Any other failure keeps everything as it was.
            if (e.status == HTTP_NOT_FOUND) forgetPin(id)
            return
        }
        val shown = OfferFlow.strip(page.sortedBy { it.createdAt }.map(::toUi), _state.value.answeredOffers)
        if (page.any { it.spoken }) SpokenThreads.mark(getApplication(), id)
        _state.update {
            if (it.threadId != id) return@update it
            // Sends the server has not acknowledged yet stay at the end.
            val unsaved = it.messages.filter { m -> m.id.startsWith("local-") && shown.none { s -> s.id == m.id } }
            val full = page.size >= HqClient.MESSAGE_PAGE
            val merged = ChatReducer.mergeLatest(it.messages, shown, full)
            it.copy(messages = merged + unsaved, hasOlder = if (merged.size > shown.size) it.hasOlder else full)
        }
    }

    private fun toUi(m: StoredMessage) = UiMessage.saved(m.id, if (m.role == "user") Speaker.User else Speaker.Reply, m.content, m.tools, m.createdAt, spoken = m.spoken, offer = m.offer)

    /**
     * Collapses the offer to its result line at once and tells HQ. Yes acts after HQ has taken the answer (or has no route for it),
     * so a failure puts the offer back instead of sending twice. Not now and Later come back on a failure too.
     */
    private fun answerOffer(id: String, answer: OfferAnswer) {
        if (!OfferFlow.canAnswer(_state.value, id)) return
        val offer = _state.value.messages.first { it.id == id }.offer ?: return
        _state.update { OfferFlow.answered(it, id, answer) }
        if (answer != OfferAnswer.Yes) startResultTimer()
        viewModelScope.launch {
            var status: Int? = null
            try {
                client.postJson("/api/offers/answer", org.json.JSONObject().put("message_id", id).put("answer", answer.wire))
            } catch (e: Exception) {
                android.util.Log.w("ChatViewModel", "could not record the answer to an offer", e)
                status = (e as? HqUnavailable)?.status ?: 0
                _state.update { OfferFlow.failed(it, id, offer, answer, status, flowTexts()) }
            }
            if (answer == OfferAnswer.Yes && OfferFlow.yesProceeds(status)) {
                val s = _state.value
                if (offer.send && s.connected && !s.running) submit(offer.reply, replaceFrom = null, files = emptyList(), contexts = emptyList()) else setDraft(offer.reply)
            }
        }
    }

    private fun flowTexts() = getApplication<Application>().let { FlowTexts(it.getString(R.string.chat_send_retry), it.getString(R.string.chat_offer_gone)) }

    /** The result line goes by itself, so the slot is free for the next thing. */
    private fun startResultTimer() {
        resultTimer?.cancel()
        resultTimer = viewModelScope.launch { delay(RESULT_MS); _state.update { it.copy(offerResult = null) } }
    }

    /** Fewer like this and Never show this on a pill. The pill leaves at once; HQ's answer decides whether to announce it or put it back. */
    private fun tellHq(capability: String, route: String, done: Int) {
        if (capability.isBlank()) return
        val before = _state.value
        _state.update { PillFlow.removed(it, capability) }
        viewModelScope.launch {
            try {
                client.postJson("/api/offers/$route", org.json.JSONObject().put("type", capability))
                _state.update { PillFlow.confirmed(it, getApplication<Application>().getString(done)) }
            } catch (e: Exception) {
                android.util.Log.w("ChatViewModel", "could not tell HQ about a pill", e)
                _state.update { PillFlow.failed(it, before, (e as? HqUnavailable)?.status ?: 0, flowTexts()) }
            }
        }
    }

    /** Pins or unpins a chat; the six pins are kept in this phone's preferences and nowhere else. */
    private fun pinChat(id: String) {
        val app = getApplication<Application>()
        when (val change = Pins.toggle(Pins.parse(pins.getString(PIN_KEY, null)), id)) {
            Pins.Change.Full -> _state.update { it.copy(notice = Notice(app.getString(R.string.chat_pin_full, Pins.MAX))) }
            is Pins.Change.Done -> {
                pins.edit().putString(PIN_KEY, Pins.encode(change.ids)).apply()
                _state.update { it.copy(chats = Pins.apply(it.chats, change.ids), notice = Notice(app.getString(if (change.pinned) R.string.chat_pinned_done else R.string.chat_unpinned_done))) }
            }
        }
    }

    private fun forgetPin(id: String) {
        val now = Pins.parse(pins.getString(PIN_KEY, null))
        if (id !in now) return
        val left = Pins.forget(now, id)
        pins.edit().putString(PIN_KEY, Pins.encode(left)).apply()
        _state.update { it.copy(chats = Pins.apply(it.chats, left)) }
    }

    /** The archive window closed without Undo (the notice went, or another archive began): the chat is gone for good, and so is its pin. */
    private fun commitArchive() {
        archiving?.first?.let(::forgetPin)
        archiving = null
    }

    /** Hides the line at once; the person is told whether it was scheduled. */
    private fun scheduleOffer(id: String, at: Long) {
        _state.update { s -> s.copy(messages = s.messages.map { if (it.id == id) it.copy(offer = null) else it }) }
        viewModelScope.launch {
            val saved = try {
                client.postJson("/api/offers/answer", org.json.JSONObject().put("message_id", id).put("answer", "scheduled").put("at", at))
                true
            } catch (e: Exception) {
                android.util.Log.w("ChatViewModel", "could not schedule an offer", e)
                false
            }
            _state.update { it.copy(notice = Notice(if (saved) OFFER_SCHEDULED else OFFER_NOT_SCHEDULED)) }
        }
    }


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
                _state.update { it.copy(openingNote = null, notice = Notice("Couldn\'t open the note. HQ didn\'t answer in time. Try again.")) }
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
                _state.update { s -> s.copy(note = s.note?.takeIf { it.path == sheet.path }?.copy(pinned = pin) ?: s.note, notice = Notice(if (pin) "Pinned to the top of the Vault" else "Unpinned")) }
            } catch (e: HqUnavailable) {
                _state.update { it.copy(notice = Notice("Couldn\'t change the pin. HQ didn\'t answer in time. Try again.")) }
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
                _state.update { it.copy(notice = Notice("Couldn\'t fetch the file. HQ didn\'t answer in time. Try again.")) }
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

    /** Starts a fresh chat with these notes attached, as "Ask HQ about these notes" does from search. */
    fun startChatWith(items: List<ContextRef>) {
        newChat()
        _state.update { it.copy(contexts = items.take(MAX_ATTACHED)) }
    }

    private fun openChats() {
        _state.update { it.copy(chatsOpen = true, chatSearch = "") }
        viewModelScope.launch { refreshChats() }
    }

    private var openedLatest = false

    private suspend fun openLatest() {
        refreshSuggestions()
        refreshChats()
        val s = _state.value
        val untouched = s.threadId == null && s.messages.isEmpty() && s.draft.isBlank() && !s.chatsOpen
        if (!openedLatest && untouched) s.chats.firstOrNull()?.let { openedLatest = true; openThread(it.id) }
    }

    /** What the empty chat offers: HQ's own pick of what works here. Keeps the last answer when hq is not reachable. */
    private suspend fun refreshSuggestions() {
        val picked = try { client.capabilitySuggestions() } catch (e: HqUnavailable) { return }
        _state.update { it.copy(suggestions = picked, pillsCleared = false) }
    }

    private suspend fun refreshChats() {
        val list = try { client.threads() } catch (e: HqUnavailable) { return }
        val spokenIds = SpokenThreads.all(getApplication())
        val rows = list.threads.map { t ->
            val group = ChatTime.group(t.updatedAt)
            val snippet = ChatPreview.snippet(t.preview) { n -> getApplication<Application>().resources.getQuantityString(R.plurals.chat_files, n, n) }
            val spoken = t.id in spokenIds
            val body = if (group == Group.Today && snippet.isNotEmpty()) "${ChatTime.age(t.updatedAt)} · $snippet" else snippet
            val subtitle = if (spoken) listOf(getApplication<Application>().getString(R.string.chat_spoken), body).filter { it.isNotEmpty() }.joinToString(" · ") else body
            ChatRow(t.id, t.title.ifBlank { ChatUiState.NEW_CHAT }, subtitle, group, t.id in list.running, t.unread > 0, spoken, hasDraft = drafts.getString(draftKey(t.id), "").orEmpty().isNotBlank())
        }
        val shown = Pins.apply(rows.filter { it.id != archiving?.first }, Pins.parse(pins.getString(PIN_KEY, null)))
        _state.update { s -> s.copy(chats = shown) }
    }

    /** The server is told at once; Undo inside the window asks it to bring the chat back. */
    private fun archive(id: String) {
        archiving?.second?.cancel()
        commitArchive()
        val app = getApplication<Application>()
        _state.update { it.copy(chats = it.chats.filter { row -> row.id != id }, notice = Notice(app.getString(R.string.undo_chat_archived), app.getString(R.string.undo))) }
        archiving = id to viewModelScope.launch {
            try { client.archive(id) } catch (e: HqUnavailable) {
                _state.update { it.copy(notice = null) }
                refreshChats()
                return@launch
            }
            if (_state.value.threadId == id) newChat(keepSheet = true)
        }
    }

    private fun undoArchive() {
        val (id, timer) = archiving ?: return
        timer.cancel()
        archiving = null
        val app = getApplication<Application>()
        viewModelScope.launch {
            val result = Undo.restore(ClientRestore(client), UndoKind.Thread, id)
            _state.update { it.copy(notice = Undo.failLine(app, result)?.let(::Notice)) }
            refreshChats()
        }
    }

    private fun draftKey(threadId: String?) = threadId ?: NEW_CHAT_KEY

    private companion object {
        const val DRAFT_PREFS = "chat_drafts"
        const val PIN_PREFS = "chat_pins"
        const val PIN_KEY = "ids"
        const val RESULT_MS = 6_000L
        const val HTTP_NOT_FOUND = 404
        const val NEW_CHAT_KEY = "new"
        const val MAX_ATTACHED = 10
        const val NOTE_GONE = "That note is not in your Vault any more"
        const val OFFER_SCHEDULED = "I will bring this back then."
        const val OFFER_NOT_SCHEDULED = "Could not schedule that. Try again."
        const val CHAT_GONE = "That chat is not in your list"
    }
}

/** [text] added after what is already typed, or alone when the field is empty; typing is never replaced. */
internal fun draftWith(now: String, text: String): String = if (now.isBlank()) text else now.trimEnd() + "\n\n" + text
