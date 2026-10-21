package example.hq.spike.ui.inbox

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqUnavailable
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
        val pending = try { client.notifications("pending") } catch (e: HqUnavailable) { null }
        if (frozen) return
        if (pending == null) {
            _state.update { it.copy(loaded = true, offline = true) }
            return
        }
        val history = try { client.notifications("history", HISTORY_LIMIT) } catch (e: HqUnavailable) { null }
        if (frozen) return
        val cards = pending.items.map(InboxWords::card)
        val needs = cards.filter { it.kind != CardKind.Update }.sortedWith(compareBy({ ORDER.indexOf(it.kind) }, { if (it.kind == CardKind.Approval) it.createdAt else "" }))
        val updates = cards.filter { it.kind == CardKind.Update }
        val done = history?.items.orEmpty().filter { it.kind == "action_needed" && (it.state == "approved" || it.state == "dismissed") }
            .map { DoneEntry(it.id, (if (it.state == "approved") "Allowed: " else "Not now: ") + InboxWords.card(it).title, it.createdAt) }
        _state.update { it.copy(loaded = true, offline = false, needs = needs, updates = updates, done = done, checkedAt = InboxWords.clock(java.time.OffsetDateTime.now().toString())) }
    }

    private fun answer(card: InboxCard, allow: Boolean) {
        if (_state.value.answering[card.id] == Answer.Sending) return
        _state.update { it.copy(answering = it.answering + (card.id to Answer.Sending)) }
        viewModelScope.launch {
            try {
                // A helper is answered in its own terminal first; the inbox row is only a record of the answer.
                if (card.kind == CardKind.HelperAsks && card.sessionId != null) client.sendSessionKeys(card.sessionId, listOf(if (allow) "enter" else "esc"))
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
        const val HISTORY_LIMIT = 30
        val ORDER = listOf(CardKind.HelperAsks, CardKind.Approval, CardKind.HelperStuck)
    }
}
