package example.hq.spike.data

import org.json.JSONObject

/** What the server broadcasts on `/ws`, for every chat. Filter on [threadId]. */
sealed interface ChatEvent {
    data class TurnStart(val threadId: String?, val clientId: String?, val userMessage: StoredMessage?, val replaceFrom: String? = null) : ChatEvent
    data class Text(val threadId: String?, val text: String) : ChatEvent
    data class Reasoning(val threadId: String?, val text: String) : ChatEvent
    data class ToolStarted(val threadId: String?, val step: ToolStep) : ChatEvent
    data class ToolProgress(val threadId: String?, val toolCallId: String, val message: String) : ChatEvent
    data class ToolFinished(val threadId: String?, val toolCallId: String, val result: String?) : ChatEvent
    data class TurnEnd(val threadId: String?, val messageId: String?, val stopped: Boolean) : ChatEvent
    data class Failed(val threadId: String?, val message: String) : ChatEvent
    data class Rejected(val clientId: String?, val threadId: String?, val reason: String, val running: Boolean) : ChatEvent
    data class Titled(val threadId: String, val title: String) : ChatEvent
    data class Lagged(val skipped: Int) : ChatEvent

    companion object {
        /** Reads one frame. Frames this app does not use (credits, other tabs' bookkeeping) give null. */
        fun parse(frame: String): ChatEvent? {
            val o = try { JSONObject(frame) } catch (e: org.json.JSONException) { return null }
            val thread = o.optStringOrNull("thread_id")
            return when (o.optString("type")) {
                "turn_start" -> TurnStart(thread, o.optStringOrNull("client_id"), o.optJSONObject("user_message")?.let(StoredMessage::from), o.optStringOrNull("replace_from"))
                "text_delta" -> if (o.optBoolean("thinking", false)) null else Text(thread, o.optString("content"))
                "reasoning_delta" -> Reasoning(thread, o.optString("content"))
                "tool_start" -> ToolStarted(thread, ToolStep(o.optString("tool_call_id"), o.optString("tool_name"), o.optStringOrNull("args"), null, null))
                "tool_progress" -> ToolProgress(thread, o.optString("tool_call_id"), o.optString("message"))
                "tool_end" -> ToolFinished(thread, o.optString("tool_call_id"), o.optStringOrNull("result"))
                "turn_end" -> TurnEnd(thread, o.optStringOrNull("message_id"), o.optBoolean("stopped", false))
                "error" -> Failed(thread, o.optString("content"))
                "chat_rejected" -> Rejected(o.optStringOrNull("client_id"), thread, o.optString("reason"), o.optBoolean("running", false))
                "thread_title" -> thread?.let { Titled(it, o.optString("title")) }
                "stream_lag" -> Lagged(o.optInt("skipped"))
                else -> null
            }
        }
    }
}
