package example.hq.spike.capture

import android.content.Context
import example.hq.spike.data.HqClient
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqUnavailable
import java.io.File
import java.io.IOException
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.runBlocking
import org.json.JSONArray
import org.json.JSONObject

/** One thing saved on the phone and not yet sent: a note or a task, with what it was made from. */
data class Queued(val id: String, val kind: String, val title: String, val payload: JSONObject, val createdAt: Long, val failure: String? = null) {
    val isNote: Boolean get() = kind == KIND_NOTE

    companion object {
        const val KIND_NOTE = "note"
        const val KIND_TASK = "task"
    }
}

/** Saved to app storage first, so a thought is never lost while HQ is down; the file is replaced whole, never edited in place. */
class Outbox(private val file: File) {
    private val lock = Any()

    fun all(): List<Queued> = synchronized(lock) { read() }

    fun add(q: Queued) = synchronized(lock) { write(trimmed(read().filter { it.id != q.id } + q)) }

    /** Over the limit, the oldest refused items go first; a thought still waiting to be sent is never dropped. */
    private fun trimmed(items: List<Queued>): List<Queued> {
        var over = items.size - MAX_ITEMS
        if (over <= 0) return items
        return items.filter { !(it.failure != null && over-- > 0) }
    }

    fun remove(id: String) = synchronized(lock) { write(read().filter { it.id != id }) }

    fun fail(id: String, why: String) = synchronized(lock) { write(read().map { if (it.id == id) it.copy(failure = why) else it }) }

    /** A failed item tried again: its failure is forgotten so the next flush sends it. */
    fun retry(id: String) = synchronized(lock) { write(read().map { if (it.id == id) it.copy(failure = null) else it }) }

    private fun read(): List<Queued> = try {
        val items = JSONObject(file.readText()).optJSONArray("items") ?: JSONArray()
        (0 until items.length()).mapNotNull { i ->
            val o = items.optJSONObject(i) ?: return@mapNotNull null
            Queued(o.getString("id"), o.getString("kind"), o.optString("title"), o.getJSONObject("payload"), o.optLong("created_at"), if (o.isNull("failure")) null else o.optString("failure"))
        }
    } catch (e: java.io.FileNotFoundException) { emptyList() } catch (e: IOException) { emptyList() } catch (e: org.json.JSONException) { setAside(); emptyList() }

    /** A file that cannot be read is kept as it is, so the next save does not write over what was in it. */
    private fun setAside() {
        runCatching { Files.move(file.toPath(), File(file.path + ".bad").toPath(), StandardCopyOption.REPLACE_EXISTING) }
    }

    private fun write(items: List<Queued>) {
        val arr = JSONArray()
        items.forEach { arr.put(JSONObject().put("id", it.id).put("kind", it.kind).put("title", it.title).put("payload", it.payload).put("created_at", it.createdAt).put("failure", it.failure ?: JSONObject.NULL)) }
        val tmp = File(file.path + ".tmp")
        java.io.FileOutputStream(tmp).use { out ->
            out.write(JSONObject().put("items", arr).toString().toByteArray())
            out.fd.sync()
        }
        Files.move(tmp.toPath(), file.toPath(), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
    }

    companion object {
        const val MAX_ITEMS = 200
        @Volatile private var shared: Outbox? = null

        fun of(c: Context): Outbox = shared ?: synchronized(this) { shared ?: Outbox(File(c.applicationContext.filesDir, "outbox.json")).also { shared = it } }
    }
}

/** What happened when one saved item was sent. */
sealed interface Sent {
    data class Done(val path: String?) : Sent
    /** HQ did not answer: the item stays and the next try is later. */
    data object Down : Sent
    /** HQ answered and would not take it: the item stays, marked, until the owner decides. */
    data class Refused(val why: String) : Sent
    /** Another send of this same item is under way. */
    data object Busy : Sent
}

object OutboxSender {
    private val running = AtomicBoolean(false)
    private val inFlight: MutableSet<String> = java.util.concurrent.ConcurrentHashMap.newKeySet()

    /** Claims an item for a send the caller makes itself ([sendHeld]), so a flush does not send it a second time. */
    fun hold(id: String) { inFlight.add(id) }

    /** One send of one item at a time: a second caller for the same item is told it is [Sent.Busy]. */
    suspend fun send(client: HqClient, q: Queued): Sent {
        if (!inFlight.add(q.id)) return Sent.Busy
        return sendHeld(client, q)
    }

    /** The send for an item already claimed with [hold]; the claim ends with it. */
    suspend fun sendHeld(client: HqClient, q: Queued): Sent = try {
        if (q.isNote) Sent.Done(client.createNote(q.payload.getString("folder"), q.payload.getString("title"), q.payload.optString("content")))
        else { client.createTask(q.payload); Sent.Done(null) }
    } catch (e: HqUnavailable) {
        when {
            e.status == 0 || e.status >= HTTP_SERVER || e.status in RETRY_LATER -> Sent.Down
            e.status == HTTP_NOT_FOUND -> Sent.Refused("HQ could not find where it goes")
            else -> Sent.Refused("HQ did not accept it")
        }
    } finally {
        inFlight.remove(q.id)
    }

    /** Sends what is waiting, oldest first, and stops at the first time HQ does not answer. Returns how many went. */
    suspend fun flush(client: HqClient, box: Outbox): Int {
        var sent = 0
        for (q in box.all().filter { it.failure == null }) {
            when (val r = send(client, q)) {
                is Sent.Done -> { box.remove(q.id); sent++ }
                Sent.Down -> return sent
                Sent.Busy -> Unit
                is Sent.Refused -> box.fail(q.id, r.why)
            }
        }
        return sent
    }

    /** For the thread that polls HQ and for a save: one flush at a time, on the caller's thread. */
    @JvmStatic
    fun flushNow(c: Context) {
        if (!running.compareAndSet(false, true)) return
        try { runBlocking { flush(HqServices.get(c.applicationContext).background, Outbox.of(c)) } }
        catch (e: Exception) { android.util.Log.w("hq-outbox", "flush stopped: " + e.javaClass.simpleName) }
        finally { running.set(false) }
    }

    private const val HTTP_SERVER = 500
    private const val HTTP_NOT_FOUND = 404
    /** Unauthorised (a token HQ has not yet accepted), timed out and too many requests all pass, so they wait. */
    private val RETRY_LATER = listOf(401, 408, 429)
}
