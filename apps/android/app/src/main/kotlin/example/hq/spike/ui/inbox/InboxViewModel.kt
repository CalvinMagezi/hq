package example.hq.spike.ui.inbox

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.data.HqServices
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
) {
    val shownNeeds: List<InboxCard> get() = needs.filter { it.id !in cleared }
    val shownUpdates: List<InboxCard> get() = updates.filter { it.id !in cleared }
}

class InboxActions(
    val onTab: (InboxTab) -> Unit = {},
    val onAllow: (InboxCard) -> Unit = {},
    val onNotNow: (InboxCard) -> Unit = {},
    val onDetails: (InboxCard?) -> Unit = {},
    val onClear: (InboxCard) -> Unit = {},
    val onClearAll: () -> Unit = {},
    val onUndo: () -> Unit = {},
    val onDismissLine: () -> Unit = {},
)

class InboxViewModel(app: Application) : AndroidViewModel(app) {
    private val client = HqServices.get(app).client
    private val _state = MutableStateFlow(InboxUi())
    val state: StateFlow<InboxUi> = _state
    private var clearing: Job? = null
    private var frozen = false

    val actions = InboxActions(
        onTab = { t -> _state.update { it.copy(tab = t) } }, onAllow = { answer(it, allow = true) }, onNotNow = { answer(it, allow = false) },
        onDetails = { c -> _state.update { it.copy(details = c) } }, onClear = { clear(listOf(it), "Update cleared") },
        onClearAll = { clear(_state.value.shownUpdates, "Updates cleared") }, onUndo = ::undo, onDismissLine = { _state.update { it.copy(line = null) } },
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

    /** How many things are waiting on the owner, for the badge on the nav bar. */
    val badge: Int get() = _state.value.shownNeeds.size

    suspend fun refresh() {
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

    private fun answer(card: InboxCard, allow: Boolean) {
        if (_state.value.answering[card.id] == Answer.Sending) return
        _state.update { it.copy(answering = it.answering + (card.id to Answer.Sending)) }
        viewModelScope.launch {
            try {
                // A helper is answered in its own terminal first, and only if its screen still shows the dialog this card quoted.
                if (card.kind == CardKind.HelperAsks && card.sessionId != null && !answerHelper(card, if (allow) "enter" else "esc")) return@launch
                client.answerNotification(card.id, if (allow) "approve" else "reject")
                _state.update { it.copy(answering = it.answering - card.id, details = null, line = InboxLine(if (allow) "Allowed." else "Not now. Nothing was done.")) }
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

    /** Hides the updates now and dismisses them after a few seconds; Undo inside that time puts them back. */
    private fun clear(cards: List<InboxCard>, text: String) {
        if (cards.isEmpty()) return
        clearing?.cancel()
        flushCleared()
        val ids = cards.map { it.id }.toSet()
        _state.update { it.copy(cleared = ids, line = InboxLine(text, "Undo")) }
        clearing = viewModelScope.launch {
            delay(UNDO_MS)
            flushCleared()
        }
    }

    private fun flushCleared() {
        val ids = _state.value.cleared
        if (ids.isEmpty()) return
        _state.update { it.copy(cleared = emptySet(), line = null, updates = it.updates.filter { u -> u.id !in ids }) }
        viewModelScope.launch {
            ids.forEach { id -> try { client.answerNotification(id, "dismiss") } catch (e: HqUnavailable) { /* it shows again on the next refresh */ } }
        }
    }

    private fun undo() {
        clearing?.cancel()
        _state.update { it.copy(cleared = emptySet(), line = null) }
    }

    private companion object {
        const val POLL_MS = 20_000L
        const val UNDO_MS = 5_000L
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
