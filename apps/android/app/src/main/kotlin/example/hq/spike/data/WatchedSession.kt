package example.hq.spike.data

import org.json.JSONObject

/** An agent session a chat watches, as `GET /api/threads/{id}/sessions` lists it. */
data class WatchedSession(
    val id: String,
    val label: String,
    val harness: String,
    /** "working", "idle", "blocked" or "done"; empty when HQ has not read it. */
    val agentStatus: String,
    /** Whether the agent's process runs; a session that ended can no longer be answered. */
    val running: Boolean,
    val drive: Boolean,
    val pendingWake: Boolean,
    val taskDisplayId: String?,
) {
    /** Waiting on someone: blocked at a prompt, or woken and not yet handled. */
    val needsAnswer: Boolean get() = running && (agentStatus == "blocked" || pendingWake)

    companion object {
        fun from(o: JSONObject) = WatchedSession(
            id = o.getString("id"),
            label = o.optStringOrNull("label").orEmpty(),
            harness = o.optString("harness", ""),
            agentStatus = o.optStringOrNull("agent_status").orEmpty(),
            running = o.optString("status", "") == "running",
            drive = o.optBoolean("drive", false),
            pendingWake = !o.isNull("pending_wake") && o.has("pending_wake"),
            taskDisplayId = o.optJSONObject("task")?.optStringOrNull("display_id"),
        )
    }
}

internal fun watchedSessions(text: String): List<WatchedSession> =
    JSONObject(text).optJSONArray("sessions").objects().map(WatchedSession::from)
