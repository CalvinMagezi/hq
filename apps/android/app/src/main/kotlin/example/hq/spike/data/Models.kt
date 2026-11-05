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
data class NoteBody(val path: String, val content: String, val truncated: Boolean, val hash: String? = null)

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
