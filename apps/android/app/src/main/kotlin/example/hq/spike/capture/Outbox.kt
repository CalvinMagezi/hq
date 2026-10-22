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

    fun add(q: Queued) = synchronized(lock) { write((read().filter { it.id != q.id } + q).takeLast(MAX_ITEMS)) }

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
    } catch (e: IOException) { emptyList() } catch (e: org.json.JSONException) { emptyList() }

    private fun write(items: List<Queued>) {
        val arr = JSONArray()
        items.forEach { arr.put(JSONObject().put("id", it.id).put("kind", it.kind).put("title", it.title).put("payload", it.payload).put("created_at", it.createdAt).put("failure", it.failure ?: JSONObject.NULL)) }
        val tmp = File(file.path + ".tmp")
        tmp.writeText(JSONObject().put("items", arr).toString())
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
}

object OutboxSender {
    private val running = AtomicBoolean(false)

    suspend fun send(client: HqClient, q: Queued): Sent = try {
        if (q.isNote) Sent.Done(client.createNote(q.payload.getString("folder"), q.payload.getString("title"), q.payload.optString("content")))
        else { client.createTask(q.payload); Sent.Done(null) }
    } catch (e: HqUnavailable) {
        when {
            e.status == 0 || e.status >= HTTP_SERVER -> Sent.Down
            e.status == HTTP_NOT_FOUND -> Sent.Refused("HQ could not find where it goes")
            else -> Sent.Refused("HQ did not accept it")
        }
    }

    /** Sends what is waiting, oldest first, and stops at the first time HQ does not answer. Returns how many went. */
    suspend fun flush(client: HqClient, box: Outbox): Int {
        var sent = 0
        for (q in box.all().filter { it.failure == null }) {
            when (val r = send(client, q)) {
                is Sent.Done -> { box.remove(q.id); sent++ }
                Sent.Down -> return sent
                is Sent.Refused -> box.fail(q.id, r.why)
            }
        }
        return sent
    }

    /** For the thread that polls HQ and for a save: one flush at a time, on the caller's thread. */
    @JvmStatic
    fun flushNow(c: Context) {
        if (!running.compareAndSet(false, true)) return
        try { runBlocking { flush(HqServices.get(c.applicationContext).client, Outbox.of(c)) } } finally { running.set(false) }
    }

    private const val HTTP_SERVER = 500
    private const val HTTP_NOT_FOUND = 404
}
