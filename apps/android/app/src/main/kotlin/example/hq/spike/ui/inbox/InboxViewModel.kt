package example.hq.spike.ui.inbox

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.NotifAccess
import example.hq.spike.R
import example.hq.spike.data.HqServices
import example.hq.spike.data.StoredMessage
import example.hq.spike.voice.VoiceTurn
import example.hq.spike.ui.shell.ClientRestore
import example.hq.spike.ui.shell.Undo
import example.hq.spike.ui.shell.UndoKind
import example.hq.spike.ui.shell.UndoResult
import example.hq.spike.data.HqUnavailable
import example.hq.spike.ui.helpers.AnswerCheck
import example.hq.spike.ui.helpers.HelperWords
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

enum class InboxTab { Needs, Updates, Done }

/** Where an answer stands on its card: being sent, or failed with a reason shown above the buttons. */
sealed interface Answer {
    data object Sending : Answer
    data class Failed(val reason: String) : Answer
}

data class DoneEntry(val id: String, val title: String, val createdAt: String)

data class InboxLine(val text: String, val action: String? = null)

/** The Needs tab's banner while notifications cannot reach the owner: what waited here, and what waits on a server HQ. */
data class InboxBanner(val waited: Int, val server: Int)

data class InboxUi(
    val loaded: Boolean = false,
    val offline: Boolean = false,
    val tab: InboxTab = InboxTab.Needs,
    val needs: List<InboxCard> = emptyList(),
    val updates: List<InboxCard> = emptyList(),
    val done: List<DoneEntry> = emptyList(),
    val answering: Map<String, Answer> = emptyMap(),
    val details: InboxCard? = null,
    /** Updates cleared a moment ago and still undoable; they are hidden but not yet dismissed on the server. */
    val cleared: Set<String> = emptySet(),
    val line: InboxLine? = null,
    val checkedAt: String = "",
    val banner: InboxBanner? = null,
    /** An approval that was just allowed: the page that says what happens next and offers Keep going. */
    val allowed: Allowed? = null,
) {
    val shownNeeds: List<InboxCard> get() = needs.filter { it.id !in cleared }
    val shownUpdates: List<InboxCard> get() = updates.filter { it.id !in cleared }
}

/** A voice approval just allowed: [at] is the time it was allowed, [sent] after Keep going has been sent. */
data class Allowed(val card: InboxCard, val at: String, val sent: Boolean = false)

class InboxActions(
    val onTab: (InboxTab) -> Unit = {},
    val onAllow: (InboxCard) -> Unit = {},
    val onNotNow: (InboxCard) -> Unit = {},
    val onDetails: (InboxCard?) -> Unit = {},
    val onClear: (InboxCard) -> Unit = {},
    val onClearAll: () -> Unit = {},
    val onUndo: () -> Unit = {},
    val onDismissLine: () -> Unit = {},
    val onBannerNotNow: () -> Unit = {},
    var onServerOpen: () -> Unit = {},
    val onKeepGoing: (Allowed) -> Unit = {},
    val onAllowedDone: () -> Unit = {},
)

class InboxViewModel(app: Application) : AndroidViewModel(app) {
    private val client = HqServices.get(app).client
    private val _state = MutableStateFlow(InboxUi())
    val state: StateFlow<InboxUi> = _state
    private var clearing: Job? = null
    private var lastCleared: List<InboxCard> = emptyList()
    private var frozen = false

    val actions = InboxActions(
        onTab = { t -> _state.update { it.copy(tab = t) } }, onAllow = { answer(it, allow = true) }, onNotNow = { answer(it, allow = false) },
        onDetails = { c -> _state.update { it.copy(details = c) } }, onClear = { clear(listOf(it), getApplication<Application>().getString(R.string.undo_update_cleared)) },
        onClearAll = { clear(_state.value.shownUpdates, "") }, onUndo = ::undo, onDismissLine = { _state.update { it.copy(line = null) } },
        onBannerNotNow = { NotifAccess.snooze(getApplication()); refreshBanner() },
        onKeepGoing = ::keepGoing, onAllowedDone = { _state.update { it.copy(allowed = null) } },
    )

    init {
        viewModelScope.launch {
            while (true) {
                if (!frozen) refresh()
                delay(POLL_MS)
            }
        }
    }

    fun freeze(fixed: InboxUi) {
        frozen = true
        _state.value = fixed
    }

    /** Opens one item on its own page, once the list has it: a notification or a link can arrive before the first read. */
    fun openItem(id: String) {
        viewModelScope.launch {
            if (_state.value.needs.none { it.id == id }) refresh()
            _state.value.needs.firstOrNull { it.id == id }?.let { card -> _state.update { it.copy(details = card) } }
        }
    }

    /** How many things are waiting on the owner, for the badge on the nav bar. */
    val badge: Int get() = _state.value.shownNeeds.size

    /** Re-reads whether the banner shows: on every poll and when the app comes back from Android settings. */
    fun refreshBanner() {
        if (frozen) return
        val c = getApplication<Application>()
        val banner = NotifAccess.banner(NotifAccess.read(c), NotifAccess.snoozeUntil(c), System.currentTimeMillis(), NotifAccess.waited(c))
        _state.update { it.copy(banner = banner) }
    }

    suspend fun refresh() {
        refreshBanner()
        val pending = try { client.notifications("pending", PENDING_LIMIT) } catch (e: HqUnavailable) { null }
        if (frozen) return
        if (pending == null) {
            _state.update { it.copy(loaded = true, offline = true) }
            return
        }
        val history = try { client.notifications("history", HISTORY_LIMIT) } catch (e: HqUnavailable) { null }
        if (frozen) return
        val lists = inboxLists(pending.items, history?.items.orEmpty())
        _state.update { it.copy(loaded = true, offline = false, needs = lists.needs, updates = lists.updates, done = lists.done, details = it.details?.let { d -> lists.needs.firstOrNull { n -> n.id == d.id } }, checkedAt = InboxWords.clock(java.time.OffsetDateTime.now().toString())) }
    }

    /** One follow-up in the same chat, marked spoken: HQ stopped to wait for the OK and this asks it to finish. */
    private fun keepGoing(a: Allowed) {
        val thread = a.card.threadId ?: return
        val words = getApplication<Application>().getString(R.string.chat_approval_followup)
        val sent = HqServices.get(getApplication()).connection.sendChat(words, thread, VoiceTurn.newClientId(), source = StoredMessage.SPOKEN)
        if (sent) _state.update { it.copy(allowed = it.allowed?.copy(sent = true)) }
    }

    private fun answer(card: InboxCard, allow: Boolean) {
        if (_state.value.answering[card.id] == Answer.Sending) return
        _state.update { it.copy(answering = it.answering + (card.id to Answer.Sending)) }
        viewModelScope.launch {
            try {
                if (card.dialog != null) {
                    // The server answers the dialog it read, and refuses one that moved on (409, handled below).
                    client.answerDialog(card.id, if (allow) "approve" else "decline")
                } else {
                    // A helper is answered in its own terminal first, and only if its screen still shows the dialog this card quoted.
                    if (card.kind == CardKind.HelperAsks && card.sessionId != null && !answerHelper(card, if (allow) "enter" else "esc")) return@launch
                    client.answerNotification(card.id, if (allow) "approve" else "reject")
                }
                val spoken = allow && card.kind == CardKind.Approval && card.quote != null && card.threadId != null
                _state.update {
                    it.copy(
                        answering = it.answering - card.id, details = null, line = InboxLine(if (allow) "Allowed." else "Not now. Nothing was done."),
                        allowed = if (spoken) Allowed(card, InboxWords.clock(java.time.OffsetDateTime.now().toString())) else it.allowed,
                    )
                }
                refresh()
            } catch (e: HqUnavailable) {
                if (e.message?.contains("HTTP 409") == true) {
                    _state.update { it.copy(answering = it.answering - card.id, details = null, line = InboxLine("Already answered.")) }
                    refresh()
                } else {
                    _state.update { it.copy(answering = it.answering + (card.id to Answer.Failed("Couldn't send your answer. HQ didn't reply. It may be restarting."))) }
                }
            }
        }
    }

    /** Sends [key] to the helper if its live screen still shows the quoted dialog; says why and returns false if not. */
    private suspend fun answerHelper(card: InboxCard, key: String): Boolean {
        val screen = client.helperScreen(card.sessionId!!, CHECK_LINES)
        val check = HelperWords.checkAnswer(HelperWords.blocked(card.excerpt), screen.lines, key, screen.live)
        if (check != AnswerCheck.Ok) {
            _state.update { it.copy(answering = it.answering + (card.id to Answer.Failed(HelperWords.checkWords(check)))) }
            refresh()
            return false
        }
        try {
            client.sendSessionKeys(card.sessionId, listOf(key))
        } catch (e: HqUnavailable) {
            if (e.message?.contains("HTTP 409") != true) throw e
            _state.update { it.copy(answering = it.answering + (card.id to Answer.Failed("That helper is no longer running."))) }
            return false
        }
        return true
    }

    /** Dismisses at once; Undo inside the window restores them on the server. */
    private fun clear(cards: List<InboxCard>, text: String) {
        if (cards.isEmpty()) return
        clearing?.cancel()
        val app = getApplication<Application>()
        val ids = cards.map { it.id }
        val line = if (ids.size == 1) text else app.resources.getQuantityString(R.plurals.updates_cleared, ids.size, ids.size)
        _state.update { it.copy(line = InboxLine(line, app.getString(R.string.undo)), updates = it.updates.filter { u -> u.id !in ids }) }
        lastCleared = cards
        clearing = viewModelScope.launch {
            ids.forEach { id -> try { client.answerNotification(id, "dismiss") } catch (e: HqUnavailable) { /* it shows again on the next refresh */ } }
            delay(Undo.millis(app))
            lastCleared = emptyList()
            _state.update { it.copy(line = null) }
        }
    }

    private fun undo() {
        clearing?.cancel()
        val cards = lastCleared
        lastCleared = emptyList()
        val app = getApplication<Application>()
        viewModelScope.launch {
            var late = false
            cards.forEach { c -> if (Undo.restore(ClientRestore(client), UndoKind.Notification, c.id) == UndoResult.TooLate) late = true }
            _state.update { it.copy(line = if (late) InboxLine(Undo.failLine(app, UndoResult.TooLate).orEmpty()) else null, updates = if (late) it.updates else (it.updates + cards).distinctBy { u -> u.id }) }
            refresh()
        }
    }

    private companion object {
        const val POLL_MS = 20_000L
        const val HISTORY_LIMIT = 200
        const val PENDING_LIMIT = 500
        /** More lines than the Helpers poll reads, so the check never shares its cached read. */
        const val CHECK_LINES = 81
    }
}

class InboxLists(val needs: List<InboxCard>, val updates: List<InboxCard>, val done: List<DoneEntry>)

private val NEEDS_ORDER = listOf(CardKind.HelperAsks, CardKind.TaskReview, CardKind.Approval, CardKind.HelperStuck)

/** What the three tabs show for what the server holds: things that need the owner, updates, and what the owner already decided. */
fun inboxLists(pending: List<example.hq.spike.data.HqClient.InboxItem>, history: List<example.hq.spike.data.HqClient.InboxItem>): InboxLists {
    val cards = pending.map(InboxWords::card)
    val needs = cards.filter { it.kind != CardKind.Update }
        .sortedWith(compareBy({ NEEDS_ORDER.indexOf(it.kind) }, { if (it.kind == CardKind.Approval || it.kind == CardKind.TaskReview) it.createdAt else "" }))
    val done = history.filter { it.kind == "action_needed" && (it.state == "approved" || it.state == "dismissed") }
        .map { DoneEntry(it.id, (if (it.state == "approved") "Allowed: " else "Not now: ") + InboxWords.card(it).title, it.createdAt) }
    return InboxLists(needs, cards.filter { it.kind == CardKind.Update }, done)
}
