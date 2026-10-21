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

/** A saved message. `meta` is present on replies that did more than talk. */
data class StoredMessage(
    val id: String,
    val threadId: String,
    val role: String,
    val content: String,
    val createdAt: String,
    val tools: List<ToolStep>,
    val stopped: Boolean,
) {
    companion object {
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
            )
        }
    }
}

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
