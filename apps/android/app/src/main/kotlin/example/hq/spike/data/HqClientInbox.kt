package example.hq.spike.data

import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject

// The inbox calls of [HqClient]: notifications, answers, session keys and the undo restores.

/** Undo's restore calls: 200 puts the item back, 409 (surfaced as [HqUnavailable.status]) means it changed or 10 minutes passed. */
suspend fun HqClient.unarchiveThread(id: String) { send(authed("/api/threads/${java.net.URLEncoder.encode(id, "UTF-8")}/unarchive").post(HqClient.EMPTY_JSON).build()) { } }

suspend fun HqClient.restoreNotification(id: String) { send(authed("/api/notifications/${java.net.URLEncoder.encode(id, "UTF-8")}/restore").post(HqClient.EMPTY_JSON).build()) { } }

/** Undoes a deleted spending limit by sending the limits as they were. */
suspend fun HqClient.restoreBudgets(limits: JSONObject) { send(authed("/api/budgets").put(json(limits)).build()) { } }

/** The inbox, newest first. [state] is pending, approved, dismissed, resolved or history; null for all. */
suspend fun HqClient.notifications(state: String?, limit: Int = 50): HqClient.InboxList =
    send(authed("/api/notifications?limit=$limit" + (state?.let { "&state=$it" } ?: "")).build()) { text ->
        val o = JSONObject(text)
        HqClient.InboxList(
            o.optJSONArray("notifications").objects().map { n ->
                val meta = n.optJSONObject("metadata")
                val extra = meta?.optJSONObject("extra")
                HqClient.InboxItem(
                    n.getString("id"), n.optString("kind", ""), n.optString("title", ""), n.optString("description", ""), n.optString("state", ""),
                    n.optString("created_at", ""), meta?.optStringOrNull("link"), extra?.optStringOrNull("source_task"), extra?.optStringOrNull("val_kind"),
                    n.optJSONArray("actions").strings(), alertDialog(meta), approvalFacts(meta),
                    signInDevice(meta), n.optBoolean("silenced", false),
                    if (n.has("owner_only")) n.optBoolean("owner_only") else null, n.optStringOrNull("source_group"), n.optStringOrNull("expired_reason"),
                    siteApprovals(n, meta), openLabel = extra?.optStringOrNull("open_label"),
                )
            },
            o.optInt("unread_count", 0), o.optInt("pending_approvals_count", 0),
            if (o.has("needs_you_owner_only")) o.optInt("needs_you_owner_only") else null,
            if (o.has("needs_you_helpers")) o.optInt("needs_you_helpers") else null,
            o.optInt("expired_count", 0),
        )
    }

/** Records the owner's answer: approve, allow_task, allow_repo, allow_starter, reject or dismiss. */
suspend fun HqClient.answerNotification(id: String, action: String, starter: String? = null, minutes: Long? = null, until: Long? = null) {
    val body = JSONObject().put("action", action)
    if (!starter.isNullOrBlank()) body.put("starter", starter)
    // A reminder's Snooze: when it rings again. `until` (epoch seconds) wins over `minutes`.
    if (minutes != null) body.put("minutes", minutes)
    if (until != null) body.put("until", until)
    send(authed("/api/notifications/${java.net.URLEncoder.encode(id, "UTF-8")}/action").post(json(body)).build()) { }
}

/**
 * Answers the dialog behind a helper's notice: [choice] is approve (the yes-once answer the server named) or decline. The
 * server sends it to that dialog only; 409 means it was answered elsewhere or the helper moved on.
 */
suspend fun HqClient.answerDialog(id: String, choice: String) {
    send(authed("/api/notifications/${java.net.URLEncoder.encode(id, "UTF-8")}/answer").post(JSONObject().put("choice", choice).toString().toRequestBody("application/json".toMediaType())).build()) { }
}

/** Types keys such as enter or esc into a helper's terminal. 409 means it is no longer at a prompt. */
suspend fun HqClient.sendSessionKeys(sessionId: String, keys: List<String>) {
    send(authed("/api/harness-sessions/${java.net.URLEncoder.encode(sessionId, "UTF-8")}/send").post(JSONObject().put("keys", JSONArray(keys)).toString().toRequestBody("application/json".toMediaType())).build()) { }
}
