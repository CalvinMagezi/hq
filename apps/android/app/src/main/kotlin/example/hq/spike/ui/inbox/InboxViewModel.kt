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

/** What the owner decided, or, with [expired], a request HQ stopped waiting on by itself, which can be restored. */
data class DoneEntry(val id: String, val title: String, val createdAt: String, val expired: Boolean = false)

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
    /** A new sign-in the owner asked to sign out: the red question shows until they answer it. */
    val askSignOut: InboxCard? = null,
    val signingOut: Boolean = false,
    val signOutError: String? = null,
    /** HQ's own count of decisions only the owner can make; null from an HQ that does not count them. */
    val ownerOnlyCount: Int? = null,
    /** The owner chose to see the decisions rather than clear the old helper requests. */
    val catchUpDismissed: Boolean = false,
    /** Done rows whose Restore is in flight: the button waits so a second tap cannot repeat it. */
    val restoring: Set<String> = emptySet(),
) {
    val shownNeeds: List<InboxCard> get() = needs.filter { it.id !in cleared }
    val shownUpdates: List<InboxCard> get() = updates.filter { it.id !in cleared }
    /** What the bar counts: the requests whose topic the owner has not silenced. */
    val badge: Int get() = ownerOnlyCount ?: shownNeeds.count { !it.silenced }

    /** The tab holds the catch-up card: more than a few helper requests, and the owner has not waved it off. */
    val catchUp: Boolean get() = !catchUpDismissed && NeedsLayout.catchUp(shownNeeds)
}

/** A voice approval just allowed: [at] is the time it was allowed, [sent] after Keep going has been sent. */
data class Allowed(val card: InboxCard, val at: String, val sent: Boolean = false)

class InboxActions(
    val onTab: (InboxTab) -> Unit = {},
    val onAllow: (InboxCard) -> Unit = {},
    val onAllowSite: (InboxCard) -> Unit = {},
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
    val onAskSignOut: (InboxCard?) -> Unit = {},
    val onSignOutConfirm: () -> Unit = {},
    val onClearHelpers: () -> Unit = {},
    val onShowOwner: () -> Unit = {},
    val onRestore: (DoneEntry) -> Unit = {},
)

class InboxViewModel(app: Application) : AndroidViewModel(app) {
    private val client = HqServices.get(app).client
    private val _state = MutableStateFlow(InboxUi())
    val state: StateFlow<InboxUi> = _state
    private val queue = ClearQueue({ id -> client.answerNotification(id, "dismiss") }, { id -> Undo.restore(ClientRestore(client), UndoKind.Notification, id) })
    private var frozen = false

    val actions = InboxActions(
        onTab = { t -> _state.update { it.copy(tab = t) } }, onAllow = { answer(it, allow = true) }, onAllowSite = { answer(it, allow = true, wholeSite = true) }, onNotNow = { answer(it, allow = false) },
        onDetails = { c -> _state.update { it.copy(details = c) } }, onClear = { clear(listOf(it)) },
        onClearAll = { clear(_state.value.shownUpdates) }, onUndo = ::undo, onDismissLine = { queue.latest()?.let { queue.expire(it) }; _state.update { it.copy(line = null) } },
        onBannerNotNow = { NotifAccess.snooze(getApplication()); refreshBanner() },
        onKeepGoing = ::keepGoing, onAllowedDone = { _state.update { it.copy(allowed = null) } },
        onAskSignOut = { c -> _state.update { it.copy(askSignOut = c, signOutError = null) } }, onSignOutConfirm = ::signOut,
        onClearHelpers = { clear(NeedsLayout.oldHelpers(_state.value.shownNeeds)) }, onShowOwner = { _state.update { it.copy(catchUpDismissed = true) } }, onRestore = ::restoreDone,
    )

    init {
        viewModelScope.launch {
            while (true) {
                if (!frozen) refresh()
                delay(POLL_MS)
            }
        }
    }

    /** Reads the inbox now rather than at the next poll: a chat that waits for an answer shows its request at once. */
    suspend fun refreshNow() {
        if (!frozen) refresh()
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
    val badge: Int get() = _state.value.badge

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
        val lists = inboxLists(pending.items, history?.items.orEmpty()).let { l -> InboxLists(withDeviceNames(l.needs), l.updates, l.done) }
        _state.update { it.copy(loaded = true, offline = false, needs = lists.needs, updates = lists.updates, done = lists.done, details = it.details?.let { d -> lists.needs.firstOrNull { n -> n.id == d.id } }, ownerOnlyCount = pending.needsOwnerOnly, checkedAt = InboxWords.clock(java.time.OffsetDateTime.now().toString())) }
    }

    /** Names each new sign-in's device from HQ's device list; one it cannot read keeps the plain "A new device". */
    private suspend fun withDeviceNames(needs: List<InboxCard>): List<InboxCard> {
        if (needs.none { it.kind == CardKind.SignIn }) return needs
        val labels = try {
            val list = client.json("/api/auth/devices").optJSONArray("devices")
            (0 until (list?.length() ?: 0)).associate { i -> list!!.getJSONObject(i).let { d -> d.optString("id") to d.optString("label") } }
        } catch (e: HqUnavailable) { emptyMap() }
        return needs.map { c -> if (c.kind == CardKind.SignIn) c.copy(deviceLabel = labels[c.signInDevice]?.ifBlank { null }) else c }
    }

    /** Signs the new sign-in's device out after the red question; HQ settles its inbox item when the device goes. */
    private fun signOut() {
        val card = _state.value.askSignOut ?: return
        val device = card.signInDevice ?: return
        if (_state.value.signingOut) return
        _state.update { it.copy(signingOut = true, signOutError = null) }
        viewModelScope.launch {
            val ctx = getApplication<Application>()
            val name = card.deviceLabel ?: ctx.getString(R.string.new_signin_unknown)
            val failed = try { client.deleteAt("/api/auth/devices/${java.net.URLEncoder.encode(device, "UTF-8")}"); false } catch (e: HqUnavailable) { e.status != HTTP_NOT_FOUND }
            _state.update {
                if (failed) it.copy(signingOut = false, signOutError = ctx.getString(R.string.fail_sign_out_no_reply, name))
                else it.copy(signingOut = false, askSignOut = null, details = null, line = InboxLine(ctx.getString(R.string.sign_out_device_done, name)))
            }
            refresh()
        }
    }

    /** One follow-up in the same chat, marked spoken: HQ stopped to wait for the OK and this asks it to finish. */
    private fun keepGoing(a: Allowed) {
        val thread = a.card.threadId ?: return
        val words = getApplication<Application>().getString(R.string.chat_approval_followup)
        val sent = HqServices.get(getApplication()).connection.sendChat(words, thread, VoiceTurn.newClientId(), source = StoredMessage.SPOKEN)
        if (sent) _state.update { it.copy(allowed = it.allowed?.copy(sent = true)) }
    }

    private fun answer(card: InboxCard, allow: Boolean, wholeSite: Boolean = false) {
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
                    client.answerNotification(card.id, if (wholeSite) "approve_site" else if (allow) "approve" else "reject")
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
                    _state.update { it.copy(answering = it.answering + (card.id to Answer.Failed("Couldn't send your answer. HQ didn't answer in time. It may be restarting."))) }
                }
            }
        }
    }

    /** Sends [key] to the helper if its live screen still shows the quoted dialog; says why and returns false if not. */
    private suspend fun answerHelper(card: InboxCard, key: String): Boolean {
        val screen = client.helperScreen(card.sessionId!!, CHECK_LINES)
        val check = HelperWords.checkAnswer(HelperWords.blocked(card.excerpt), screen.lines, key, screen.live)
        if (check != AnswerCheck.Ok) {
            _state.update { it.copy(answering = it.answering + (card.id to Answer.Failed(HelperWords.checkWords(example.hq.spike.ui.shell.Words.of(getApplication<Application>()), check)))) }
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

    /** Hides the cards at once and dismisses them in the background, one clear after another; Undo restores what was dismissed. */
    private fun clear(cards: List<InboxCard>) {
        if (cards.isEmpty()) return
        val app = getApplication<Application>()
        val ids = cards.flatMap { it.ids }
        val line = app.resources.getQuantityString(ClearWords.plural(cards), cards.size, cards.size)
        _state.update { it.copy(line = InboxLine(line, app.getString(R.string.undo)), updates = it.updates.filter { u -> u.id !in ids }, needs = it.needs.filter { n -> n.id !in ids }) }
        val batch = queue.begin(cards)
        viewModelScope.launch { queue.run(batch) }
    }

    private fun undo() {
        val app = getApplication<Application>()
        viewModelScope.launch {
            val undone = queue.undo() ?: return@launch
            val cards = undone.cards
            _state.update {
                it.copy(
                    line = if (undone.late) InboxLine(Undo.failLine(app, UndoResult.TooLate).orEmpty()) else null,
                    updates = (it.updates + cards.filter { c -> c.kind == CardKind.Update }).distinctBy { u -> u.id },
                    needs = (it.needs + cards.filter { c -> c.kind != CardKind.Update }).distinctBy { n -> n.id },
                )
            }
            refresh()
        }
    }

    /** Puts back a request HQ stopped waiting on. A refusal (HQ's 409) says it is too late, in the same words as Undo. */
    private fun restoreDone(entry: DoneEntry) {
        if (entry.id in _state.value.restoring) return
        val app = getApplication<Application>()
        _state.update { it.copy(restoring = it.restoring + entry.id) }
        viewModelScope.launch {
            val result = Undo.restore(ClientRestore(client), UndoKind.Notification, entry.id)
            _state.update { it.copy(restoring = it.restoring - entry.id, line = Undo.failLine(app, result)?.let { text -> InboxLine(text) }) }
            refresh()
        }
    }

    private companion object {
        const val POLL_MS = 20_000L
        const val HISTORY_LIMIT = 200
        const val PENDING_LIMIT = 500
        const val HTTP_NOT_FOUND = 404
        /** More lines than the Helpers poll reads, so the check never shares its cached read. */
        const val CHECK_LINES = 81
    }
}

class InboxLists(val needs: List<InboxCard>, val updates: List<InboxCard>, val done: List<DoneEntry>)

internal val NEEDS_ORDER = listOf(CardKind.SignIn, CardKind.HelperAsks, CardKind.TaskReview, CardKind.Approval, CardKind.HelperStuck)

/** What the three tabs show for what the server holds: things that need the owner, updates, and what the owner already decided. */
fun inboxLists(pending: List<example.hq.spike.data.HqClient.InboxItem>, history: List<example.hq.spike.data.HqClient.InboxItem>): InboxLists {
    val cards = pending.map(InboxWords::card)
    val needs = cards.filter { it.kind != CardKind.Update }
        .sortedWith(compareBy({ NEEDS_ORDER.indexOf(it.kind) }, { if (it.kind == CardKind.Approval || it.kind == CardKind.TaskReview) it.createdAt else "" }))
    val done = history.filter { it.kind == "action_needed" && (it.state == "approved" || it.state == "dismissed" || it.expiredReason != null) }
        .map {
            val title = InboxWords.card(it).title
            if (it.expiredReason != null) DoneEntry(it.id, title, it.createdAt, expired = true)
            else DoneEntry(it.id, (if (it.state == "approved") "Allowed: " else "Not now: ") + title, it.createdAt)
        }
    return InboxLists(needs, foldUpdates(cards.filter { it.kind == CardKind.Update }), done)
}

private const val SAME_EVENT_SECS = 60L

/** One row per event: updates with the same words within a minute are one event. The row keeps every id, so clearing it clears them all. */
fun foldUpdates(updates: List<InboxCard>): List<InboxCard> {
    val kept = mutableListOf<InboxCard>()
    for (u in updates) {
        val at = example.hq.spike.data.Instants.parse(u.createdAt)
        val i = kept.indexOfFirst { o ->
            val other = example.hq.spike.data.Instants.parse(o.createdAt)
            o.title == u.title && o.body == u.body && at != null && other != null && kotlin.math.abs(java.time.Duration.between(at, other).seconds) <= SAME_EVENT_SECS
        }
        if (i < 0) kept += u else kept[i] = kept[i].copy(alsoIds = kept[i].alsoIds + u.ids)
    }
    return kept
}
