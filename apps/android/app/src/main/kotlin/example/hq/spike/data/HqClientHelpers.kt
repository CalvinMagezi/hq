package example.hq.spike.data

import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject

// The helper calls of [HqClient]: the list, its screens, starting, following and stopping.

/** The agent sessions this chat watches. */
suspend fun HqClient.watchedSessions(threadId: String): List<WatchedSession> =
    send(authed("/api/threads/${java.net.URLEncoder.encode(threadId, "UTF-8")}/sessions").build()) { example.hq.spike.data.watchedSessions(it) }

/** Turns Drive on or off for a watched session and returns it as saved. 409 means HQ refused to turn it on; 404 that it is no longer watched. */
suspend fun HqClient.setSessionDrive(id: String, drive: Boolean): WatchedSession =
    send(authed("/api/harness-sessions/${java.net.URLEncoder.encode(id, "UTF-8")}/drive").post(JSONObject().put("drive", drive).toString().toRequestBody("application/json".toMediaType())).build()) {
        WatchedSession.from(JSONObject(it).getJSONObject("session"))
    }

/** Stops the chat watching a session; the server also switches its Drive off. */
suspend fun HqClient.unwatchSession(id: String) {
    send(authed("/api/harness-sessions/${java.net.URLEncoder.encode(id, "UTF-8")}/unwatch").post(HqClient.EMPTY_JSON).build()) { }
}

/** Every helper, newest first, including stopped ones. */
suspend fun HqClient.helpers(): List<HqClient.HelperRow> = send(authed("/api/harness-sessions").build()) { helperRows(it) }

suspend fun HqClient.helperScreen(id: String, lines: Int = HqClient.SCREEN_LINES): HqClient.HelperScreen =
    send(authed("/api/harness-sessions/${java.net.URLEncoder.encode(id, "UTF-8")}/screen?lines=$lines").build()) {
        val o = JSONObject(it)
        HqClient.HelperScreen(o.optJSONArray("lines").strings(), o.optString("source", "") == "live")
    }

/** Types a line into a helper. 409 means it is waiting at a prompt and must be answered there first. */
suspend fun HqClient.sendHelperText(id: String, text: String) {
    send(authed("/api/harness-sessions/${java.net.URLEncoder.encode(id, "UTF-8")}/send").post(JSONObject().put("text", text).toString().toRequestBody("application/json".toMediaType())).build()) { }
}

suspend fun HqClient.stopHelper(id: String) {
    send(authed("/api/harness-sessions/${java.net.URLEncoder.encode(id, "UTF-8")}/stop").post(HqClient.EMPTY_JSON).build()) { }
}

/** Opens the chat that follows this helper, making one if no chat does yet; returns that chat's id. Asking again returns the same chat. */
suspend fun HqClient.followHelper(id: String): String =
    send(authed("/api/harness-sessions/${java.net.URLEncoder.encode(id, "UTF-8")}/adopt").post(HqClient.EMPTY_JSON).build()) { JSONObject(it).optString("thread_id") }

suspend fun HqClient.renameHelper(id: String, label: String) {
    send(authed("/api/harness-sessions/${java.net.URLEncoder.encode(id, "UTF-8")}/rename").post(JSONObject().put("label", label).toString().toRequestBody("application/json".toMediaType())).build()) { }
}

/** Starts a helper in a new folder on [host], given its first instruction; returns the new session's id. */
suspend fun HqClient.startHelper(harness: String, host: String, prompt: String, label: String, note: String? = null, taskId: String? = null, repo: String? = null): String =
    send(
        authed("/api/harness-sessions").post(JSONObject().put("harness", harness).put("host", host).put("prompt", prompt).put("label", label).put("note", note).put("task_id", taskId).put("repo", repo).toString().toRequestBody("application/json".toMediaType())).build(),
        caller = startHttp,
    ) { JSONObject(it).getString("session_id") }

/** The computers helpers can run on, and whether each answered. */
suspend fun HqClient.helperHosts(): List<HqClient.HelperHost> = send(authed("/api/workbench/hosts").build()) { text ->
    val root = JSONObject(text)
    val every = root.optJSONArray("harnesses").strings()
    root.optJSONArray("hosts").objects().mapNotNull { h -> h.optStringOrNull("host")?.let { HqClient.HelperHost(it, h.optBoolean("reachable", false), h.optStringOrNull("error"), h.optJSONArray("agents")?.strings() ?: every) } }
}
