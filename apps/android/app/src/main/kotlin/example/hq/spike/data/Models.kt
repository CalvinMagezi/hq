package example.hq.spike.data

import org.json.JSONArray
import org.json.JSONObject

/** One chat thread as `GET /api/threads` returns it. */
data class ChatThread(
    val id: String,
    val title: String,
    val updatedAt: String,
    val preview: String?,
    val unread: Int,
) {
    companion object {
        fun from(o: JSONObject) = ChatThread(
            id = o.getString("thread_id"),
            title = o.optString("title", ""),
            updatedAt = o.optString("updated_at", ""),
            preview = o.optStringOrNull("last_message_preview"),
            unread = o.optInt("unread_count", 0),
        )
    }
}

/** A tool call a reply made, from the saved `meta.tool_steps` or from live `tool_start` and `tool_end` frames. */
data class ToolStep(val id: String, val name: String, val args: String?, val result: String?, val durationMs: Long?)

/** Something HQ offers to do under a reply (`meta.offer`), chosen on the device; the model never sees it. Yes sends [reply], or puts it in the composer when [send] is false. */
data class Offer(val kind: String, val line: String, val reply: String, val send: Boolean) {
    companion object {
        fun from(o: JSONObject?): Offer? {
            if (o == null) return null
            val kind = o.optString("kind", "")
            val line = o.optString("line", "")
            val reply = o.optString("reply", "")
            if (kind.isBlank() || line.isBlank() || reply.isBlank()) return null
            return Offer(kind, line, reply, o.optBoolean("send", false))
        }
    }
}

/** A saved message. `meta` is present on replies that did more than talk. */
data class StoredMessage(
    val id: String,
    val threadId: String,
    val role: String,
    val content: String,
    val createdAt: String,
    val tools: List<ToolStep>,
    val stopped: Boolean,
    /** HQ's own mark that the question was spoken (`meta.source`), and the id the sender gave it. */
    val source: String? = null,
    val clientId: String? = null,
    val offer: Offer? = null,
) {
    val spoken: Boolean get() = role == "user" && source == SPOKEN

    companion object {
        const val SPOKEN = "voice"

        fun from(o: JSONObject): StoredMessage {
            val meta = o.optJSONObject("meta")
            return StoredMessage(
                id = o.getString("message_id"),
                threadId = o.optString("thread_id", ""),
                role = o.getString("role"),
                content = o.optString("content", ""),
                createdAt = o.optString("created_at", ""),
                tools = meta?.optJSONArray("tool_steps").toToolSteps(),
                stopped = meta?.optBoolean("stopped", false) ?: false,
                source = meta?.optString("source", "")?.ifBlank { null },
                clientId = meta?.optString("client_id", "")?.ifBlank { null },
                offer = Offer.from(meta?.optJSONObject("offer")),
            )
        }
    }
}

/** A note's text as saved, front matter included. [truncated] when hq cut a very long note. */
data class NoteBody(
    val path: String, val content: String, val truncated: Boolean, val hash: String? = null,
    val writtenBy: String = WRITTEN_BY_YOU, val review: String? = null, val why: String? = null, val tags: List<String> = emptyList(),
)

data class ThreadList(val threads: List<ChatThread>, val running: Set<String>)

private fun JSONArray?.toToolSteps(): List<ToolStep> {
    if (this == null) return emptyList()
    return (0 until length()).map { i ->
        val s = getJSONObject(i)
        ToolStep(
            id = s.optString("id", ""),
            name = s.optString("name", ""),
            args = s.optStringOrNull("args"),
            result = s.optStringOrNull("result"),
            durationMs = if (s.has("duration_ms") && !s.isNull("duration_ms")) s.getLong("duration_ms") else null,
        )
    }
}

internal fun JSONObject.optStringOrNull(key: String): String? = if (has(key) && !isNull(key)) getString(key) else null

/** Who wrote a note's current version, as HQ marks it. */
const val WRITTEN_BY_YOU = "you"
const val WRITTEN_BY_HQ = "hq"

/** Where a restored note went (`restored_to`). */
const val RESTORED_ORIGINAL = "original"
const val RESTORED_FOLDER = "folder"
const val RESTORED_VAULT = "vault"

/** The query string of a note search: the words, how many, and the three narrowings HQ knows. */
internal fun noteSearchQuery(query: String, limit: Int, folder: String?, writtenByHq: Boolean, tag: String?): String {
    fun enc(v: String) = java.net.URLEncoder.encode(v, "UTF-8")
    return buildString {
        append("q=").append(enc(query)).append("&limit=").append(limit)
        folder?.let { append("&folder=").append(enc(it)) }
        if (writtenByHq) append("&written_by=hq")
        tag?.takeIf { it.isNotBlank() }?.let { append("&tag=").append(enc(it.trimStart('#'))) }
    }
}

/** Reads what HQ says about a note without trusting its shape: an absent, null or odd field falls back to today's behaviour. */
internal object NoteWire {
    private val WRITERS = setOf(WRITTEN_BY_YOU, WRITTEN_BY_HQ, "import")
    private val RESTORED = setOf(RESTORED_ORIGINAL, RESTORED_FOLDER, RESTORED_VAULT)

    /** A string field that may be missing, null or blank; those read as absent. */
    fun text(o: JSONObject, key: String): String? = if (!o.has(key) || o.isNull(key)) null else (o.opt(key) as? String)?.takeIf { it.isNotBlank() }

    fun writtenBy(o: JSONObject): String = text(o, "written_by")?.takeIf { it in WRITERS } ?: WRITTEN_BY_YOU

    /** Only `new` and `changed` mean anything; any other word is no mark. */
    fun review(o: JSONObject): String? = text(o, "review")?.takeIf { it == "new" || it == "changed" }

    /** The tags that are strings; a null or a number among them is dropped. */
    fun tags(o: JSONObject): List<String> {
        val a = o.optJSONArray("tags") ?: return emptyList()
        return (0 until a.length()).mapNotNull { (a.opt(it) as? String)?.trim()?.takeIf(String::isNotEmpty) }
    }

    fun restoredTo(o: JSONObject): String = text(o, "restored_to")?.takeIf { it in RESTORED } ?: RESTORED_ORIGINAL

    fun item(o: JSONObject): HqClient.NoteItem = HqClient.NoteItem(
        o.getString("path"), o.optString("title", ""), o.optString("preview", ""), o.optString("mtime", ""), o.optBoolean("pinned", false),
        writtenBy(o), review(o), text(o, "why"), tags(o),
    )

    fun body(o: JSONObject, path: String): NoteBody = NoteBody(
        o.optString("path", path), o.optString("content", ""), o.optBoolean("truncated", false), o.optString("hash", "").ifBlank { null },
        writtenBy(o), review(o), text(o, "why"), tags(o),
    )
}
