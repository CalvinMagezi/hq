package example.hq.spike.ui.helpers

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.data.HqClient.HelperHost
import example.hq.spike.data.HqClient.HelperRow
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqUnavailable
import java.time.LocalDateTime
import java.time.ZoneOffset
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

enum class HelperFilter { All, Needs, Working, Finished }

/** Which page a helper is open on: its own page, or the question it is waiting on. */
enum class HelperPage { Detail, Ask }

data class StartForm(
    val job: String = "",
    val harness: String = "claude-code",
    val host: String? = null,
    val hosts: List<HelperHost>? = null,
    val busy: Boolean = false,
    val error: String? = null,
)

data class HelperOpen(
    val id: String,
    val page: HelperPage,
    val screen: List<String> = emptyList(),
    val draft: String = "",
    val sending: Boolean = false,
    val answering: Boolean = false,
    val error: String? = null,
    val menu: Boolean = false,
    val confirmStop: Boolean = false,
    val rename: String? = null,
    val raw: Boolean = false,
    val technical: Boolean = false,
)

data class HelperLine(val text: String)

data class HelpersUi(
    val loaded: Boolean = false,
    val offline: Boolean = false,
    val rows: List<HelperRow> = emptyList(),
    /** What each blocked helper is asking, read from its screen. */
    val asks: Map<String, BlockedPrompt> = emptyMap(),
    /** The last words on each working helper's screen, for the line under its title. */
    val lastWords: Map<String, String> = emptyMap(),
    val filter: HelperFilter = HelperFilter.All,
    val finishedOpen: Boolean = false,
    val open: HelperOpen? = null,
    val start: StartForm? = null,
    val line: HelperLine? = null,
    val now: LocalDateTime = LocalDateTime.now(ZoneOffset.UTC),
) {
    val cards: List<HelperCard> get() = rows.filter { !it.archived }.map { HelperWords.card(it, asks[it.id], now, lastWords[it.id]) }.sortedWith(HelperWords.order)
    fun count(filter: HelperFilter) = cards.count { matches(it, filter) }
    val openCard: HelperCard? get() = open?.let { o -> cards.firstOrNull { it.id == o.id } }
}

fun matches(card: HelperCard, filter: HelperFilter) = when (filter) {
    HelperFilter.All -> true
    HelperFilter.Needs -> card.state == HelperState.NeedsOk
    HelperFilter.Working -> card.state == HelperState.Working
    HelperFilter.Finished -> card.state == HelperState.Finished || card.state == HelperState.Stopped
}

class HelpersActions(
    val onFilter: (HelperFilter) -> Unit = {},
    val onToggleFinished: () -> Unit = {},
    val onOpen: (String) -> Unit = {},
    val onClose: () -> Unit = {},
    val onAllow: (String) -> Unit = {},
    val onNotNow: (String) -> Unit = {},
    val onToggleRaw: () -> Unit = {},
    val onSeeHelper: () -> Unit = {},
    val onSeeAsk: () -> Unit = {},
    val onDraft: (String) -> Unit = {},
    val onSend: () -> Unit = {},
    val onQuick: (String) -> Unit = {},
    val onKey: (String) -> Unit = {},
    val onMenu: (Boolean) -> Unit = {},
    val onTechnical: (Boolean) -> Unit = {},
    val onAskStop: (Boolean) -> Unit = {},
    val onStop: () -> Unit = {},
    val onRename: (String?) -> Unit = {},
    val onSaveRename: () -> Unit = {},
    val onStartOpen: () -> Unit = {},
    val onStartClose: () -> Unit = {},
    val onJob: (String) -> Unit = {},
    val onHarness: (String) -> Unit = {},
    val onNextHost: () -> Unit = {},
    val onStartSubmit: () -> Unit = {},
    val onDismissLine: () -> Unit = {},
)

class HelpersViewModel(app: Application) : AndroidViewModel(app) {
    private val client = HqServices.get(app).client
    private val _state = MutableStateFlow(HelpersUi())
    val state: StateFlow<HelpersUi> = _state
    private var frozen = false
    private var screenJob: Job? = null
    private var lineJob: Job? = null
    private val answering = mutableSetOf<String>()
    private val refreshLock = Mutex()
    @Volatile private var generation = 0
    /** For each helper answered from here: the dialog that was answered, hidden until the screen shows a different one. */
    private val answered = mutableMapOf<String, List<String>>()

    /** The number on the Helpers tab: helpers waiting for the owner. */
    val waiting: Int get() = _state.value.count(HelperFilter.Needs)

    val actions = HelpersActions(
        onFilter = { f -> _state.update { it.copy(filter = f) } },
        onToggleFinished = { _state.update { it.copy(finishedOpen = !it.finishedOpen) } },
        onOpen = ::open,
        onClose = ::close,
        onAllow = { answer(it, "enter", "Allowed once.") },
        onNotNow = { answer(it, "esc", "Not now. It waits.") },
        onToggleRaw = { editOpen { it.copy(raw = !it.raw) } },
        onSeeHelper = { editOpen { it.copy(page = HelperPage.Detail) } },
        onSeeAsk = { editOpen { it.copy(page = HelperPage.Ask) } },
        onDraft = { t -> editOpen { it.copy(draft = t, error = null) } },
        onSend = ::sendDraft,
        onQuick = ::sendText,
        onKey = ::sendKey,
        onMenu = { m -> editOpen { it.copy(menu = m) } },
        onTechnical = { t -> editOpen { it.copy(technical = t, menu = false) } },
        onAskStop = { a -> editOpen { it.copy(confirmStop = a, menu = false) } },
        onStop = ::stop,
        onRename = { r -> editOpen { it.copy(rename = r?.let(HelperWords::oneLineLabel), menu = false) } },
        onSaveRename = ::saveRename,
        onStartOpen = ::openStart,
        onStartClose = { _state.update { it.copy(start = null) } },
        onJob = { j -> _state.update { s -> s.copy(start = s.start?.copy(job = j, error = null)) } },
        onHarness = { h -> _state.update { s -> s.copy(start = s.start?.copy(harness = h)) } },
        onNextHost = ::nextHost,
        onStartSubmit = ::submitStart,
        onDismissLine = { _state.update { it.copy(line = null) } },
    )

    init {
        viewModelScope.launch {
            while (true) {
                if (!frozen) refresh()
                delay(POLL_MS)
            }
        }
    }

    fun freeze(fixed: HelpersUi) {
        frozen = true
        _state.value = fixed
    }

    /** Opens a helper that is waiting for the owner on its question; used when an alert or a card points here. */
    fun openAsking() {
        _state.value.cards.firstOrNull { it.state == HelperState.NeedsOk }?.let { open(it.id) }
    }

    suspend fun refresh() = refreshLock.withLock {
        val started = generation
        val rows = try { client.helpers() } catch (e: HqUnavailable) { null }
        if (frozen) return@withLock
        if (rows == null) {
            _state.update { it.copy(loaded = true, offline = true) }
            return@withLock
        }
        val live = rows.filter { it.status == "running" && it.reachable && it.alive != false }
        val screens = live.filter { it.agentStatus == "blocked" || it.agentStatus == "working" }.associate { r ->
            r.id to try { client.helperScreen(r.id).lines } catch (e: HqUnavailable) { emptyList() }
        }
        // An answer finished while this read was out, so what it saw is older than the answer: drop it, the next poll is fresh.
        if (frozen || started != generation) return@withLock
        val read = screens.mapNotNull { (id, lines) -> rows.first { it.id == id }.takeIf { it.agentStatus == "blocked" }?.let { HelperWords.blocked(lines)?.let { a -> id to a } } }.toMap()
        val asks = HelperWords.visibleAsks(read, answered)
        val words = screens.mapNotNull { (id, lines) -> HelperWords.lastWords(lines)?.let { id to it } }.toMap()
        _state.update { it.copy(loaded = true, offline = false, rows = rows, asks = asks, lastWords = words, now = LocalDateTime.now(ZoneOffset.UTC)) }
        val open = _state.value.open
        if (open != null && rows.none { it.id == open.id && !it.archived }) close()
    }

    private fun editOpen(change: (HelperOpen) -> HelperOpen) = _state.update { s -> s.open?.let { s.copy(open = change(it)) } ?: s }

    private fun say(text: String) {
        lineJob?.cancel()
        _state.update { it.copy(line = HelperLine(text)) }
        lineJob = viewModelScope.launch { delay(LINE_MS); _state.update { it.copy(line = null) } }
    }

    private fun open(id: String) {
        val card = _state.value.cards.firstOrNull { it.id == id } ?: return
        val page = if (card.state == HelperState.NeedsOk && card.ask != null) HelperPage.Ask else HelperPage.Detail
        _state.update { it.copy(open = HelperOpen(id, page)) }
        watchScreen(id)
    }

    private fun close() {
        screenJob?.cancel()
        _state.update { it.copy(open = null) }
    }

    /** Keeps the open helper's screen fresh while its page is showing. */
    private fun watchScreen(id: String) {
        screenJob?.cancel()
        screenJob = viewModelScope.launch {
            while (true) {
                val lines = try { client.helperScreen(id).lines } catch (e: HqUnavailable) { null }
                if (lines != null && !frozen) editOpen { if (it.id == id) it.copy(screen = lines) else it }
                delay(SCREEN_MS)
            }
        }
    }

    /**
     * Sends enter or esc to a helper that is waiting, but only after a fresh read of its live screen shows the very dialog the
     * owner was shown (every line of it, cursor included), and, for enter, a highlighted choice that is a plain yes.
     */
    private fun answer(id: String, key: String, done: String) {
        if (!answering.add(id)) return
        if (_state.value.open?.id == id) editOpen { it.copy(answering = true, error = null) }
        viewModelScope.launch {
            try {
                val shown = _state.value.asks[id]
                val screen = client.helperScreen(id)
                val check = if (screen.live) HelperWords.checkAnswer(shown, screen.lines, key) else null
                when (check) {
                    AnswerCheck.Ok -> {
                        client.sendSessionKeys(id, listOf(key))
                        generation++
                        shown?.let { answered[id] = it.block }
                        if (_state.value.open?.id == id) { screenJob?.cancel(); _state.update { it.copy(open = null) } }
                        say(done)
                        refresh()
                    }
                    null -> refuse(id, "Couldn't read its screen just now. Look again before you answer.")
                    AnswerCheck.NoPrompt, AnswerCheck.Changed -> refuse(id, "It asked something else just now. Look again before you answer.")
                    AnswerCheck.BroadYes, AnswerCheck.NoCursor -> refuse(id, "Its highlighted choice would say yes for good, or cannot be read. Open the helper to pick the answer.")
                }
            } catch (e: HqUnavailable) {
                if (e.message?.contains("HTTP 409") == true) { closeIfOpen(id); say("That helper is no longer running."); refresh() }
                else tell(id, "Couldn't send your answer. HQ didn't reply. It may be restarting.")
            } finally {
                answering.remove(id)
            }
        }
    }

    private fun closeIfOpen(id: String) {
        if (_state.value.open?.id == id) { screenJob?.cancel(); _state.update { it.copy(open = null) } }
    }

    private suspend fun refuse(id: String, why: String) {
        tell(id, why)
        refresh()
    }

    /** Shows [text] on the open helper's page, or as a line when the answer came from a card on the list. */
    private fun tell(id: String, text: String) {
        if (_state.value.open?.id == id) editOpen { it.copy(answering = false, error = text) } else say(text)
    }

    private fun sendDraft() {
        val text = _state.value.open?.draft?.trim().orEmpty()
        if (text.isNotEmpty()) sendText(text)
    }

    private fun sendText(text: String) {
        val id = _state.value.open?.id ?: return
        if (_state.value.open?.sending == true) return
        editOpen { it.copy(sending = true, error = null) }
        viewModelScope.launch {
            try {
                client.sendHelperText(id, text)
                editOpen { it.copy(sending = false, draft = "") }
                say("Sent.")
            } catch (e: HqUnavailable) {
                val conflict = e.message?.contains("HTTP 409") == true
                val waiting = _state.value.cards.firstOrNull { it.id == id }?.state == HelperState.NeedsOk
                val why = when { !conflict -> "Couldn't send that. HQ didn't reply."; waiting -> "It is waiting for your OK first."; else -> "That helper is no longer running." }
                editOpen { it.copy(sending = false, error = why) }
            }
        }
    }

    private fun sendKey(key: String) {
        val id = _state.value.open?.id ?: return
        viewModelScope.launch {
            try { client.sendSessionKeys(id, listOf(key)) } catch (e: HqUnavailable) { editOpen { it.copy(error = "Couldn't send that key. HQ didn't reply.") } }
        }
    }

    private fun stop() {
        val id = _state.value.open?.id ?: return
        viewModelScope.launch {
            try {
                client.stopHelper(id)
                screenJob?.cancel()
                _state.update { it.copy(open = null) }
                say("Stopped.")
                refresh()
            } catch (e: HqUnavailable) {
                editOpen { it.copy(confirmStop = false, error = "Couldn't stop it. HQ didn't reply.") }
            }
        }
    }

    private fun saveRename() {
        val o = _state.value.open ?: return
        val name = o.rename?.trim().orEmpty()
        if (name.isEmpty()) { editOpen { it.copy(rename = null) }; return }
        viewModelScope.launch {
            try {
                client.renameHelper(o.id, HelperWords.oneLineLabel(name))
                editOpen { it.copy(rename = null) }
                refresh()
            } catch (e: HqUnavailable) {
                editOpen { it.copy(rename = null, error = "Couldn't rename it. HQ didn't reply.") }
            }
        }
    }

    private fun openStart() {
        _state.update { it.copy(start = StartForm()) }
        viewModelScope.launch {
            val hosts = try { client.helperHosts() } catch (e: HqUnavailable) { emptyList() }
            _state.update { s -> s.start?.let { f -> s.copy(start = f.copy(hosts = hosts, host = f.host ?: hosts.firstOrNull { it.reachable }?.name)) } ?: s }
        }
    }

    private fun nextHost() = _state.update { s ->
        val f = s.start ?: return@update s
        val reachable = f.hosts.orEmpty().filter { it.reachable }
        if (reachable.size < 2) s else s.copy(start = f.copy(host = reachable[(reachable.indexOfFirst { it.name == f.host } + 1) % reachable.size].name))
    }

    private fun submitStart() {
        val f = _state.value.start ?: return
        val host = f.host
        if (f.busy || f.job.isBlank() || host == null) return
        _state.update { it.copy(start = f.copy(busy = true, error = null)) }
        viewModelScope.launch {
            try {
                val id = client.startHelper(f.harness, host, f.job.trim(), HelperWords.oneLineLabel(f.job))
                _state.update { it.copy(start = null) }
                say("Started.")
                refresh()
                open(id)
            } catch (e: HqUnavailable) {
                _state.update { s -> s.copy(start = s.start?.copy(busy = false, error = "Couldn't start it. ${startReason(e)}")) }
            }
        }
    }

    private fun startReason(e: HqUnavailable) = when {
        e.message?.contains("HTTP 409") == true -> "That computer is not ready for another helper."
        e.message?.contains("HTTP 404") == true -> "This edition of HQ cannot start helpers."
        e.message?.contains("HTTP 400") == true -> "HQ did not accept that text. Try shorter, plain words."
        else -> "HQ didn't reply."
    }

    private companion object {
        const val POLL_MS = 8_000L
        const val SCREEN_MS = 5_000L
        const val LINE_MS = 4_000L
    }
}
