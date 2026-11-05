package example.hq.spike.voice

import android.Manifest
import android.app.NotificationManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import example.hq.spike.AlertPoster
import example.hq.spike.R
import example.hq.spike.data.ChatEvent
import example.hq.spike.data.HqServices
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject

/** What the handoff asks HQ. Threads and their saved messages answer after the socket has dropped or events were missed. */
interface HandoffApi {
    suspend fun running(): Set<String>
    /** The thread a question landed in, found by the client id it was sent with. */
    suspend fun threadOf(clientId: String): String?
    suspend fun lastReply(threadId: String): String?
}

/** Where the open questions and the last answer are kept across a process death. */
interface HandoffStore {
    fun load(): String?
    fun save(text: String)
}

/** What the owner is told. Each returns whether a notice was actually posted (no permission means none). */
interface HandoffNotices {
    fun answered(threadId: String, answer: String?): Boolean
    fun needsOk(threadId: String): Boolean
    fun failed(threadId: String?, reason: String): Boolean
}

/** A closed sheet's question that HQ is still working on. */
data class Pending(val clientId: String, val threadId: String?, val sentAt: Long)

/**
 * Takes over a question when the sheet closes while HQ is still working. One watcher per question, keyed by its client
 * id: it ends on the turn's own end, a failure, or the thread no longer running, and never on silence alone. It asks HQ
 * again after a reconnect or missed events, polls the running list while quiet, and gives up after [capMs].
 */
class VoiceHandoff(
    private val events: SharedFlow<ChatEvent>,
    private val connected: StateFlow<Boolean>,
    private val api: HandoffApi,
    private val store: HandoffStore,
    private val notices: HandoffNotices,
    private val scope: CoroutineScope,
    private val now: () -> Long = System::currentTimeMillis,
    private val pollMs: Long = POLL_MS,
    private val firstPollMs: Long = FIRST_POLL_MS,
    private val capMs: Long = CAP_MS,
    private val failureLine: (String) -> String = { it },
) {
    private val jobs = HashMap<String, Job>()
    private val lock = Any()

    /** Starts watching [pending] and keeps it on disk until it ends. */
    fun adopt(pending: Pending) {
        synchronized(lock) {
            if (jobs.containsKey(pending.clientId)) return
            writePending(readPending() + pending)
            jobs[pending.clientId] = scope.launch { watch(pending) }
        }
    }

    /** Picks up every question that was open when the process last died. */
    fun restore() = readPending().forEach { p -> synchronized(lock) { if (!jobs.containsKey(p.clientId)) jobs[p.clientId] = scope.launch { watch(p) } } }

    private enum class End { Answered, NeedsOk, Failed, Stopped, GaveUp }

    private suspend fun watch(start: Pending) {
        var thread = start.threadId
        var reason: String? = null
        val result = kotlinx.coroutines.CompletableDeferred<End>()
        suspend fun check() {
            val id = thread ?: api.threadOf(start.clientId).also { thread = it } ?: return
            if (id !in api.running()) result.complete(End.Answered)
        }
        val listening = scope.launch(Dispatchers.Unconfined) {
            events.collect { e ->
                when (e) {
                    is ChatEvent.TurnStart -> if (e.clientId == start.clientId) thread = e.threadId
                    is ChatEvent.TurnEnd -> if (e.threadId == thread) result.complete(if (e.waitingForOwner) End.NeedsOk else End.Answered)
                    is ChatEvent.Failed -> if (e.threadId == thread) { reason = e.message; result.complete(End.Failed) }
                    is ChatEvent.Rejected -> if (e.clientId == start.clientId) { reason = e.reason; result.complete(End.Failed) }
                    is ChatEvent.Lagged -> scope.launch { runCatching { check() } }
                    else -> Unit
                }
            }
        }
        val redo = scope.launch { connected.drop(1).collect { if (it) runCatching { check() } } }
        val polling = scope.launch {
            delay(firstPollMs)
            while (true) { runCatching { check() }; delay(pollMs) }
        }
        val capped = scope.launch { delay(capMs); result.complete(End.GaveUp) }
        val end = result.await()
        listOf(listening, redo, polling, capped).forEach { it.cancel() }
        finish(start, thread, end, reason)
    }

    private suspend fun finish(start: Pending, thread: String?, end: End, reason: String?) {
        synchronized(lock) { jobs.remove(start.clientId); writePending(readPending().filter { it.clientId != start.clientId }) }
        val id = thread
        when (end) {
            End.Stopped, End.GaveUp -> Unit
            End.NeedsOk -> if (id != null) record(id, notices.needsOk(id), answered = false)
            End.Failed -> notices.failed(id, failureLine(reason.orEmpty()))
            End.Answered -> if (id != null) record(id, notices.answered(id, runCatching { api.lastReply(id) }.getOrNull()), answered = true)
        }
    }

    private fun record(thread: String, posted: Boolean, answered: Boolean) {
        if (!answered) return
        val o = JSONObject(store.load() ?: "{}")
        o.put("last", JSONObject().put("thread", thread).put("at", now()).put("posted", posted))
        store.save(o.toString())
    }

    /** The thread whose answer came after the sheet closed with no notice posted and no look at the chat, within an hour. */
    fun unseenAnswer(): String? {
        val last = JSONObject(store.load() ?: "{}").optJSONObject("last") ?: return null
        if (last.optBoolean("posted") || last.optBoolean("seen")) return null
        return if (now() - last.optLong("at") <= UNSEEN_MS) last.optString("thread") else null
    }

    /** Opening the chat (from the sheet or the app) clears the next-open line. */
    fun markSeen() {
        val o = JSONObject(store.load() ?: return)
        o.optJSONObject("last")?.put("seen", true) ?: return
        store.save(o.toString())
    }

    private fun readPending(): List<Pending> {
        val a = JSONObject(store.load() ?: return emptyList()).optJSONArray("pending") ?: return emptyList()
        return (0 until a.length()).map { a.getJSONObject(it) }.filter { now() - it.optLong("at") < capMs }
            .map { Pending(it.getString("client"), it.optString("thread").ifBlank { null }, it.optLong("at")) }
    }

    private fun writePending(list: List<Pending>) {
        val o = JSONObject(store.load() ?: "{}")
        o.put("pending", JSONArray(list.map { JSONObject().put("client", it.clientId).put("thread", it.threadId ?: "").put("at", it.sentAt) }))
        store.save(o.toString())
    }

    companion object {
        const val POLL_MS = 30_000L
        const val FIRST_POLL_MS = 180_000L
        const val CAP_MS = 30 * 60_000L
        const val UNSEEN_MS = 3_600_000L
        const val ANSWER_CHARS = 160
        private const val FILE = "voice_handoff.json"

        @Volatile private var instance: VoiceHandoff? = null

        /** The one handoff for this process, wired to the running HQ. */
        @JvmStatic
        fun get(context: Context): VoiceHandoff = instance ?: synchronized(this) {
            instance ?: build(context.applicationContext).also { instance = it }
        }

        /** Called when the hub starts: questions left open by a dead process get a watcher again. */
        @JvmStatic
        fun restore(context: Context) = get(context).restore()

        private fun build(c: Context): VoiceHandoff {
            val services = HqServices.get(c)
            return VoiceHandoff(
                services.connection.events, services.connection.connected, ClientApi(services.background), FileStore(java.io.File(c.noBackupFilesDir, FILE)),
                AndroidNotices(c), CoroutineScope(kotlinx.coroutines.SupervisorJob() + Dispatchers.Default),
            )
        }
    }
}

private class ClientApi(private val client: example.hq.spike.data.HqClient) : HandoffApi {
    override suspend fun running() = client.threads().running

    override suspend fun threadOf(clientId: String): String? {
        val list = client.threads()
        return list.threads.take(THREADS_LOOKED_AT).firstOrNull { t -> client.messages(t.id, limit = RECENT).any { it.clientId == clientId } }?.id
    }

    override suspend fun lastReply(threadId: String): String? = client.messages(threadId, limit = RECENT).lastOrNull { it.role != "user" }?.content

    private companion object {
        const val THREADS_LOOKED_AT = 5
        const val RECENT = 10
    }
}

private class FileStore(private val file: java.io.File) : HandoffStore {
    override fun load(): String? = runCatching { file.readText() }.getOrNull()
    override fun save(text: String) { runCatching { file.writeText(text) } }
}

/** Posts the notices of spec 04: generic by default, the first 160 characters only with details on and the phone unlocked. */
private class AndroidNotices(private val c: Context) : HandoffNotices {
    private fun allowed() = Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
        c.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    private fun post(key: String, channel: String, title: String, body: String, thread: String?): Boolean {
        if (!allowed()) return false
        val link = thread?.let { "hq://chat/$it" } ?: "hq://chat"
        val n = AlertPoster.simpleNotice(c, channel, title, body, AlertPoster.open(c, link))
        c.getSystemService(NotificationManager::class.java).notify(AlertPoster.notificationId("voice:$key"), n)
        return true
    }

    override fun answered(threadId: String, answer: String?): Boolean {
        val details = AlertPoster.detailsNow(c) && !answer.isNullOrBlank()
        val body = if (details) answer!!.trim().take(VoiceHandoff.ANSWER_CHARS) else c.getString(R.string.notice_answered_body)
        return post(threadId, AlertPoster.UPDATES, c.getString(R.string.notice_answered_title), body, threadId)
    }

    override fun needsOk(threadId: String) =
        post(threadId, AlertPoster.NEEDS, c.getString(R.string.notice_voice_ok_title), c.getString(R.string.notice_open_to_see_body), threadId)

    override fun failed(threadId: String?, reason: String) =
        post(threadId ?: "none", AlertPoster.UPDATES, c.getString(R.string.voice_notice_failed_title), reason, threadId)
}
