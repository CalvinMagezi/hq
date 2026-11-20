package example.hq.spike.data

import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import example.hq.spike.Loopback
import example.hq.spike.ServerProof
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import example.hq.spike.ui.chat.blocks.Attachment
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONArray
import org.json.JSONObject

/** The local hq is not answering, or answered with an error. */
/** hq did not answer, or answered with an error; [status] is the HTTP status when there was one. */
class HqUnavailable(
    message: String,
    cause: Throwable? = null,
    val status: Int = 0,
    val serverMessage: String? = null,
) : IOException(message, cause) {
    companion object {
        /** HQ answered, but with something the phone cannot read: an error to retry, not "HQ is not running". */
        const val UNREADABLE = -1
    }
}

/** hq cannot make that export from this note (no tables, a file that is not Markdown); [message] says why. */
class ExportRefused(message: String) : IOException(message)

/** An export saved in the app's cache. */
data class ExportedFile(val file: java.io.File, val name: String, val mime: String)

/** hq answered, and the note is not there (or the path is not one it will serve). */
class NoteNotFound(path: String) : IOException("No note at $path")

/** HQ refused a save because the note changed after it was opened; [theirsHash] is what it is now. */
class NoteChanged(val theirsHash: String?, val by: String?, val at: String?) : IOException("The note changed somewhere else")

/** The same REST and web socket API the web app uses, over loopback. Auth is the sealed web token. */
class HqClient(private val token: () -> String, private val base: String = LOCAL_HQ, private val onSignedOut: () -> Unit = {}, private val callSeconds: Long = 0) {
    /** The sealed token is decrypted once, off the main thread, and forgotten when hq refuses it. */
    @Volatile private var bearer: String? = null

    /** Starting a helper answers only once the agent has settled, and an export of a large note is made before it is sent; both can take longer than any other call here. */
    /** A shrink decodes and re-encodes every picture, about 25 seconds for a 3 MB document on a Galaxy A32. */
    internal val suiteHttp by lazy { http.newBuilder().readTimeout(SUITE_READ_SECONDS, TimeUnit.SECONDS).callTimeout(0, TimeUnit.SECONDS).build() }
    internal val startHttp by lazy { http.newBuilder().readTimeout(START_READ_SECONDS, TimeUnit.SECONDS).build() }

    internal val http = OkHttpClient.Builder()
        .connectTimeout(CONNECT_SECONDS, TimeUnit.SECONDS)
        .readTimeout(READ_SECONDS, TimeUnit.SECONDS)
        .pingInterval(PING_SECONDS, TimeUnit.SECONDS)
        .apply { if (callSeconds > 0) callTimeout(callSeconds, TimeUnit.SECONDS) }
        // A carrier APN proxy cannot reach this phone's loopback (see Loopback).
        .apply { Loopback.proxyFor(java.net.URL(base))?.let { proxy(it) } }
        // No request carries the token to the phone's own hq until the server there has proved it holds that token
        // (ServerProof): another app could be listening on the port while hq is not.
        .addInterceptor(okhttp3.Interceptor { chain ->
            val request = chain.request()
            val auth = request.header("Authorization")
            if (auth != null && auth.startsWith(BEARER) && !ServerProof.ensure(base, auth.removePrefix(BEARER))) {
                throw HqUnavailable("hq has not shown it is the phone's HQ yet")
            }
            try {
                chain.proceed(request)
            } catch (e: java.net.ConnectException) {
                ServerProof.forget()
                throw e
            }
        })
        .build()

    suspend fun healthy(): Boolean = withContext(Dispatchers.IO) {
        try {
            http.newCall(Request.Builder().url("$base/health").build()).execute().use { it.isSuccessful }
        } catch (e: IOException) {
            false
        }
    }

    /** The AI service state from HQ's signed-in route, never `/health`; null when HQ does not answer it (an older HQ, or down). */
    suspend fun aiStatus(): AiStatus? = try {
        send(authed("/api/status/ai").build()) { AiStatus.parse(it) }
    } catch (e: HqUnavailable) {
        null
    }

    /** HQ's own time zone, from the open /api/hello route; null when HQ does not answer or names no zone. */
    suspend fun hello(): HqZone? = withContext(Dispatchers.IO) {
        try {
            http.newCall(Request.Builder().url("$base/api/hello").build()).execute().use { r -> if (r.isSuccessful) HqZone.parse(r.body?.string().orEmpty()) else null }
        } catch (e: IOException) {
            null
        }
    }

    suspend fun threads(limit: Int = THREAD_PAGE): ThreadList = send(authed("/api/threads?limit=$limit").build()) { body ->
        val o = JSONObject(body)
        ThreadList(o.getJSONArray("threads").objects().map(ChatThread::from), o.optJSONArray("running").strings().toSet())
    }

    /** Oldest first within the page. Pass the oldest id you have as [before] for the page above it. */
    suspend fun messages(threadId: String, before: String? = null, limit: Int = MESSAGE_PAGE): List<StoredMessage> {
        val query = "limit=$limit" + (before?.let { "&before=$it" } ?: "")
        return send(authed("/api/threads/$threadId/messages?$query").build()) { body ->
            JSONArray(body).objects().map(StoredMessage::from)
        }
    }

    /** Stop over HTTP, for when the socket is not open to carry the frame. */
    suspend fun stopThread(threadId: String) {
        send(authed("/api/threads/${java.net.URLEncoder.encode(threadId, "UTF-8")}/stop").post(EMPTY_JSON).build()) { }
    }

    suspend fun archive(threadId: String) {
        send(authed("/api/threads/$threadId/archive").post(EMPTY_JSON).build()) { }
    }

    /** A task as the lists and the detail page need it. Dates are `YYYY-MM-DD`; the ready time is UTC without a zone. */
    data class TaskItem(
        val id: String,
        val displayId: String,
        val title: String,
        val description: String,
        val status: String,
        val priority: String?,
        val dueDate: String?,
        val startDate: String?,
        val tags: List<String>,
        val assignees: List<String>,
        val parentId: String?,
        val subtaskCount: Int,
        val subtaskDone: Int,
        val blockedReason: String?,
        val readyAt: String?,
        val updatedAt: String,
        val blockedSince: String? = null,
        val waitingOn: String? = null,
        val completedAt: String? = null,
        val archivedAt: String? = null,
        val initiativeId: String? = null,
    )

    /** A helper working on a task right now: who, where, and which task. */
    data class WorkSession(val taskId: String, val actor: String, val harness: String, val cwd: String)

    data class TaskComment(val id: String, val author: String, val body: String, val createdAt: String)

    internal fun taskOf(o: JSONObject) = TaskItem(
        o.getString("id"), o.optString("display_id", ""), o.optString("title", ""), o.optString("description", ""), o.optString("status", "to_do"),
        o.optStringOrNull("priority"), o.optStringOrNull("due_date"), o.optStringOrNull("start_date"), o.optJSONArray("tags").strings(), o.optJSONArray("assignees").strings(),
        o.optStringOrNull("parent_task_id"), o.optInt("subtask_count", 0), o.optInt("subtask_done", 0), o.optStringOrNull("blocked_reason"),
        o.optStringOrNull("first_ready_for_review_at"), o.optString("updated_at", ""),
        o.optStringOrNull("blocked_since"), o.optStringOrNull("waiting_on"), o.optStringOrNull("completed_at"), o.optStringOrNull("archived_at"),
        o.optStringOrNull("initiative_id"),
    )

    internal fun json(body: JSONObject) = body.toString().toRequestBody("application/json".toMediaType())

    /** One row of the owner's inbox: an approval, a helper waiting, or an update. [actions] says what can be answered. */
    data class InboxItem(
        val id: String,
        val kind: String,
        val title: String,
        val description: String,
        val state: String,
        val createdAt: String,
        val link: String?,
        val sourceTask: String?,
        val valKind: String?,
        val actions: List<String>,
        /** For a helper's question the server read: the dialog it is about. Null from an older server. */
        val dialog: AlertDialog? = null,
        /** For an owner approval: the call HQ will make, and whether HQ says it loses something. Null from an older server. */
        val approval: ApprovalFacts? = null,
        /** For a new sign-in: the device that signed in, which Sign it out signs out. */
        val signInDevice: String? = null,
        /** Its topic is silenced: listed, but left off the badge. False from an older server. */
        val silenced: Boolean = false,
        /** Only the owner can decide it. Null from an HQ that does not say, which the phone treats as today. */
        val ownerOnly: Boolean? = null,
        /** Who it is from: `you`, `helpers` or `system`. Null from an older HQ. */
        val sourceGroup: String? = null,
        /** Why it left Needs you by itself (`auto_tidy:<rule>`); set only on the items HQ tidied away. */
        val expiredReason: String? = null,
        /** How many times the owner already allowed this site in a chat; 0 when HQ does not say. */
        val siteApprovals: Int? = null,
        /** Set only on a Later offer's item: the item can bring the offer back (action `open`), and this is the button's word. */
        val openLabel: String? = null,
    )

    /** What HQ says an approval is about: [tool] and [args] verbatim, [loses] and [verb] (one of [LOSING_VERBS]) decided by HQ, never by the card's words. */
    data class ApprovalFacts(val tool: String, val args: String, val loses: Boolean, val verb: String?, val threadId: String? = null, val site: String? = null)

    /**
     * The dialog a helper's alert is about. [approveLabel] names the one answer a button may give (a plain yes, once),
     * null when none may; the server answers it through `/api/notifications/{id}/answer`, so a button never picks the
     * wrong dialog or a "don't ask again" option.
     */
    data class AlertDialog(
        val sessionId: String,
        val seq: Long,
        val kind: String,
        val question: String?,
        val command: String?,
        val approveLabel: String?,
        val answered: Boolean,
        val loses: Boolean = false,
        val verb: String? = null,
    )

    data class InboxList(
        val items: List<InboxItem>,
        val unread: Int,
        val pending: Int,
        /** The waiting decisions only the owner can make, and the helper requests the agents handle; null from an older HQ. */
        val needsOwnerOnly: Int? = null,
        val needsHelpers: Int? = null,
        /** Items the tidy moved to Done that can still be restored. */
        val expiredCount: Int = 0,
    )

    /** A helper (a coding agent in a terminal on one of the owner's computers), as the list route sends it. */
    data class HelperRow(
        val id: String,
        val harness: String,
        val label: String,
        val host: String,
        val cwd: String,
        val status: String,
        val agentStatus: String,
        val lastSeenAt: String,
        val createdAt: String,
        val goal: String?,
        val taskTitle: String?,
        /** Null when the computer did not answer, so nothing is known about the agent. */
        val alive: Boolean?,
        val reachable: Boolean,
        val detail: String?,
        val archived: Boolean,
        /** The dialog HQ raised an alert for, if the helper waits on one; the server answers it, the screen is not parsed. */
        val decision: HelperDecision? = null,
    )

    data class HelperOption(val id: Int, val label: String, val remember: Boolean, val typed: Boolean)

    data class HelperDecision(val itemId: String, val seq: Long, val options: List<HelperOption>, val question: String?) {
        /** The id the notification routes take for this dialog's inbox item. */
        val notificationId: String get() = "val_$itemId"
    }

    /** [agents] is what this computer can start; an older computer that cannot say lists every agent HQ knows. */
    data class HelperHost(val name: String, val reachable: Boolean, val error: String?, val agents: List<String> = emptyList())

    /** One run of HQ's own edge worker: the chat that worked a task, and the task as it is now. */
    data class EdgeRun(
        val askId: String,
        val threadId: String,
        /** `pending` while it works, then `answered` or `failed`. */
        val status: String,
        val startedAt: String,
        val finishedAt: String?,
        val taskId: String,
        val displayId: String?,
        val title: String?,
        val taskStatus: String?,
        val job: EdgeJob? = null,
        val prUrl: String? = null,
        val notes: List<String> = emptyList(),
    )

    /** What a coding job may spend and has spent, and where it runs. */
    data class EdgeJob(val repo: String, val attempts: Int, val budgetUsd: Double, val spentUsd: Double, val model: String?)

    /** Whether the edge worker takes tasks, the tag that queues one, the repositories it may change, and its newest runs. */
    data class EdgeWorker(val enabled: Boolean, val tag: String, val repos: List<String>, val runs: List<EdgeRun>, val taskBudgetUsd: Double = 0.0)

    /** One repository the person's GitHub App can reach, as HQ read it. */
    data class GithubRepo(val repo: String, val flagged: Boolean, val picked: Boolean, val secrets: List<String>, val previews: List<String>, val hasCi: Boolean, val protected: Boolean)

    /** Where the GitHub connection stands: `not_connected`, `needs_install`, `connected` or `reconnect`. */
    data class GithubLink(val standing: String, val owner: String?, val selection: String?, val installUrl: String?, val settingsUrl: String?, val firstTaskConfirmed: Boolean, val repos: List<GithubRepo>)

    suspend fun github(): GithubLink = send(authed("/api/github").build()) { text ->
        val o = JSONObject(text)
        val repos = o.optJSONArray("repos").objects().map { row ->
            val r = row.optJSONObject("report") ?: JSONObject()
            GithubRepo(r.optString("repo"), row.optBoolean("flagged"), row.optBoolean("picked"), r.optJSONArray("secrets").strings(), r.optJSONArray("previews").strings(), r.optBoolean("has_ci"), r.optBoolean("protected"))
        }
        GithubLink(o.optString("standing"), o.optStringOrNull("owner"), o.optStringOrNull("repository_selection"), o.optStringOrNull("install_url"), o.optStringOrNull("settings_url"), o.optBoolean("first_task_confirmed"), repos)
    }

    suspend fun edgeWorker(): EdgeWorker = send(authed("/api/edge-worker").build()) { text ->
        val o = JSONObject(text)
        val runs = o.optJSONArray("runs").objects().map { r ->
            val t = r.optJSONObject("task")
            EdgeRun(
                r.optString("ask_id"), r.optString("thread_id"), r.optString("status"), r.optString("started_at"), r.optStringOrNull("finished_at"),
                r.optString("task_id"), t?.optStringOrNull("display_id"), t?.optStringOrNull("title"), t?.optStringOrNull("status"),
                r.optJSONObject("job")?.let { j -> EdgeJob(j.optString("repo"), j.optInt("attempts"), j.optDouble("budget_usd", 0.0), j.optDouble("spent_usd", 0.0), j.optStringOrNull("model")) },
                r.optStringOrNull("pr_url"), r.optJSONArray("notes").strings(),
            )
        }
        EdgeWorker(o.optBoolean("enabled"), o.optString("tag", "edge-task"), o.optJSONArray("repos").strings(), runs, o.optDouble("task_budget_usd", 0.0))
    }

    /** What a helper's terminal shows: its last lines, and whether they were read from the live terminal or are an old snapshot. */
    data class HelperScreen(val lines: List<String>, val live: Boolean)

    /** A read-only JSON route as it is, for the Settings pages that only show what the server says. */
    /** The empty chat's chips: what works on this install, untried things first. */
    suspend fun capabilitySuggestions(): List<example.hq.spike.ui.chat.Suggestion> =
        send(authed("/api/capabilities").build()) { example.hq.spike.ui.chat.Suggestions.parse(it) }

    suspend fun json(path: String): JSONObject = send(authed(path).build()) { JSONObject(it) }

    /** What a server HQ says about its backups at `/api/backup/status`. [lastOkMs] is epoch milliseconds, null when it never finished one. */
    data class ServerBackup(val configured: Boolean, val lastOkMs: Long?, val failed: Boolean, val overdue: Boolean, val everyHours: Int)

    /** The server's backup report, or null when it does not say: an older server without the route, or any failure. */
    suspend fun backupStatus(): ServerBackup? = try {
        send(authed("/api/backup/status").build()) { text ->
            val o = JSONObject(text)
            ServerBackup(o.optBoolean("configured"), parseWhen(o.optString("last_ok_at", "")), o.optBoolean("failed"), o.optBoolean("overdue"), o.optInt("every_hours"))
        }
    } catch (e: IOException) { null } catch (e: org.json.JSONException) { null }

    private fun parseWhen(text: String): Long? = text.takeIf { it.isNotBlank() }?.let {
        Instants.parse(it)?.toEpochMilli()
    }

    /** Notes in a list: path, title, a short preview, when it changed and whether it is pinned. */
    data class NoteItem(
        val path: String, val title: String, val preview: String, val mtime: String, val pinned: Boolean,
        /** Who wrote this version: you, hq or import. An older HQ sends nothing, which reads as you. */
        val writtenBy: String = WRITTEN_BY_YOU,
        /** `new` or `changed` while the owner has not looked at what HQ wrote; null otherwise. */
        val review: String? = null,
        val why: String? = null,
        val tags: List<String> = emptyList(),
    )

    internal fun noteItems(array: JSONArray?) = array.objects().map(NoteWire::item)

    /** The home screen's Recent list and whether HQ filed something the owner has not seen (the dot on Vault). */
    data class VaultSignals(val recent: List<NoteItem>, val hasNew: Boolean)

    /** One folder's contents: sub-folders with the notes under each, and the notes directly inside. */
    data class FolderListing(val folders: List<Pair<String, Int>>, val notes: List<String>)

    /** Full-text matches with a snippet of the text around the match. */
    data class NoteHit(val path: String, val title: String, val notebook: String, val snippet: String)

    /** What HQ checked and finished in the last [hours] hours, as the web's While you were away reads it; null when this HQ has no verification (404). */
    suspend fun verificationBriefing(hours: Int = 24): JSONObject? = try {
        send(authed("/api/verification/briefing?hours=$hours").build()) { JSONObject(it) }
    } catch (e: HqUnavailable) {
        if (e.status == HTTP_NOT_FOUND) null else throw e
    }

    /** A POST with a JSON body (empty by default) whose answer is a JSON object, or none. */
    suspend fun postJson(path: String, body: JSONObject = JSONObject()): JSONObject = send(authed(path).post(json(body)).build()) { if (it.isBlank()) JSONObject() else JSONObject(it) }

    suspend fun deleteAt(path: String) { send(authed(path).delete().build()) { } }

    suspend fun deleteJson(path: String): JSONObject = send(authed(path).delete().build()) { if (it.isBlank()) JSONObject() else JSONObject(it) }

    /** [byHq] is true when HQ moved the note to the Trash itself; an HQ that does not say reads as the owner. */
    data class TrashedNote(val trashPath: String, val originalPath: String, val trashedOn: String, val daysLeft: Int, val byHq: Boolean = false)

    /** Where a restored note went: its own folder, the nearest folder that is left, or the top of the Vault. An older HQ does not say, which reads as its own folder. */
    data class Restored(val path: String, val to: String)

    /** Checks an AI provider key with one small call to the provider and, if it works, stores it. 400 means the key was refused, 409 that the environment sets it. */
    suspend fun setProviderKey(provider: String, key: String) {
        send(authed("/api/settings/provider-key").post(JSONObject().put("provider", provider).put("api_key", key).toString().toRequestBody("application/json".toMediaType())).build()) { }
    }

    /** A new empty chat with a title, as `/topic <name>` makes. */
    suspend fun createThread(title: String): ChatThread =
        send(authed("/api/threads").post(JSONObject().put("title", title).toString().toRequestBody("application/json".toMediaType())).build()) { ChatThread.from(JSONObject(it)) }

    /**
     * Saves a file for the next message; the reply carries its vault path and is sent back with the message.
     * The file is streamed from [open], not read into memory, and the upload stops past [maxBytes].
     */
    suspend fun uploadFile(name: String, mime: String, size: Long, maxBytes: Long, open: () -> java.io.InputStream?): Attachment {
        val body = object : okhttp3.RequestBody() {
            override fun contentType() = mime.ifBlank { "application/octet-stream" }.toMediaTypeOrNull()
            override fun contentLength() = size
            override fun writeTo(sink: okio.BufferedSink) {
                val input = open() ?: throw IOException("The file could not be read")
                input.use {
                    val buffer = ByteArray(UPLOAD_CHUNK)
                    var total = 0L
                    while (true) {
                        val n = it.read(buffer)
                        if (n < 0) break
                        total += n
                        if (total > maxBytes) throw IOException("The file is over the limit")
                        sink.write(buffer, 0, n)
                    }
                }
            }
        }
        return send(authed("/api/chat/uploads?name=${java.net.URLEncoder.encode(name, "UTF-8")}").post(body).build()) { text ->
            val o = JSONObject(text)
            Attachment(o.getString("name"), o.getString("path"), o.optString("mime", ""), o.optLong("size", size))
        }
    }

    /** Top-level tasks, newest first, optionally filtered by words: (display id, title, status). */
    suspend fun searchTasks(query: String, limit: Int = PICKER_PAGE): List<Triple<String, String, String>> =
        send(authed("/api/tasks?top_level_only=true&limit=$limit&search=${java.net.URLEncoder.encode(query, "UTF-8")}").build()) { text ->
            JSONObject(text).optJSONArray("tasks").objects().map { Triple(it.getString("display_id"), it.optString("title", ""), it.optString("status", "")) }
        }

    /** Tells HQ the phone has the device action; the id is the only thing in the request. */
    suspend fun ackDeviceAction(actionId: String) {
        send(authed("/api/device-actions/${java.net.URLEncoder.encode(actionId, "UTF-8")}/ack").post(EMPTY_JSON).build()) { }
    }

    /** Reports what the phone did. [result] is the contract's outcome object, with no identity in it. */
    suspend fun postDeviceActionResult(actionId: String, result: JSONObject) {
        send(authed("/api/device-actions/${java.net.URLEncoder.encode(actionId, "UTF-8")}/result").post(result.toString().toRequestBody("application/json".toMediaType())).build()) { }
    }

    /** The device actions still open for this session, so one whose socket dropped after the insert can still be settled. */
    suspend fun pendingDeviceActions(): List<ChatEvent.DeviceAction> = send(authed("/api/device-actions/pending").build()) { body ->
        val root = body.trim()
        val list = if (root.startsWith("[")) JSONArray(root) else JSONObject(root).optJSONArray("actions") ?: JSONArray()
        (0 until list.length()).mapNotNull { list.optJSONObject(it)?.let(ChatEvent::deviceAction) }
    }

    suspend fun wsTicket(): String = send(authed("/api/ws-ticket").post(EMPTY_JSON).build()) { JSONObject(it).getString("ticket") }

    fun openSocket(ticket: String, listener: WebSocketListener): WebSocket =
        http.newWebSocket(Request.Builder().url("${base.replaceFirst("http", "ws")}/ws?ticket=$ticket").build(), listener)

    /** The server wants this header on requests that change a helper, setup or a credential; sending it on every request is harmless. */
    internal fun authed(path: String) = Request.Builder().url(base + path).header("Authorization", "Bearer ${bearerToken()}").header("X-HQ-Client", "android")

    /** Reads the sealed token now, so the first call from a screen does not wait on the Keystore. */
    fun warmUp() { runCatching { bearerToken() } }

    /** hq refused the token: forget it and let the hub replace it if the owner signed it out. */
    internal fun refused() {
        bearer = null
        onSignedOut()
    }

    private fun bearerToken(): String = bearer ?: try {
        token().also { bearer = it }
    } catch (e: IllegalStateException) {
        throw HqUnavailable("the web token is not available", e)
    }

    internal suspend fun <T> send(request: Request, caller: OkHttpClient = http, parse: (String) -> T): T = withContext(Dispatchers.IO) {
        try {
            var response = caller.newCall(request).execute()
            if (response.code == HTTP_UNAUTHORIZED) {
                refused()
                // Something else (the alert poller) may have replaced the token already; that one gets a single try.
                val fresh = "Bearer ${bearerToken()}"
                if (fresh != request.header("Authorization")) {
                    response.close()
                    response = caller.newCall(request.newBuilder().header("Authorization", fresh).build()).execute()
                }
            }
            response.use { r ->
                val body = r.body.string()
                if (!r.isSuccessful) {
                    val serverMsg = runCatching { JSONObject(body).optString("error", "").trim().ifBlank { null } }.getOrNull()
                    throw HqUnavailable("HTTP ${r.code} for ${request.url.encodedPath}", status = r.code, serverMessage = serverMsg)
                }
                parse(body)
            }
        } catch (e: IOException) {
            throw if (e is HqUnavailable) e else HqUnavailable("hq is not reachable", e)
        } catch (e: org.json.JSONException) {
            throw HqUnavailable("hq sent something unreadable", e, status = HqUnavailable.UNREADABLE)
        }
    }

    /** The file name a download asks for: the UTF-8 form when there is one, else the plain one; never a path. */
    internal fun fileNameOf(disposition: String?): String? {
        val d = disposition ?: return null
        val raw = Regex("filename\\*=UTF-8''([^;]+)").find(d)?.groupValues?.get(1)?.let { java.net.URLDecoder.decode(it, "UTF-8") }
            ?: Regex("filename=\"([^\"]+)\"").find(d)?.groupValues?.get(1)
        return raw?.substringAfterLast('/')?.replace('\\', '_')?.takeIf { it.isNotBlank() && it != ".." }
    }

    companion object {
        val LOCAL_HQ = "http://127.0.0.1:${example.hq.spike.BuildConfig.HQ_PORT}"
        internal const val HTTP_NOT_FOUND = 404
        internal const val HTTP_CONFLICT = 409
        internal const val HTTP_BAD_REQUEST = 400
        internal const val HTTP_UNAUTHORIZED = 401
        internal const val HTTP_UNPROCESSABLE = 422
        private const val SUITE_READ_SECONDS = 300L
        private const val BEARER = "Bearer "
        private const val CONNECT_SECONDS = 3L
        private const val READ_SECONDS = 20L
        private const val START_READ_SECONDS = 150L
        private const val PING_SECONDS = 20L
        private const val THREAD_PAGE = 50
        internal const val PICKER_PAGE = 20
        internal const val TASK_PAGE = 500
        internal const val TASK_CAP = 5_000
        internal const val SCREEN_LINES = 80
        private const val UPLOAD_CHUNK = 64 * 1024
        const val MESSAGE_PAGE = 100
        internal val EMPTY_JSON = "{}".toRequestBody("application/json".toMediaType())
    }
}

/** The dialog in a notification's metadata, or null for any other notice. */
/** The device a new sign-in item is about, from `metadata.sign_in.device_id`; read the same way by the Inbox and the shade. */
internal fun signInDevice(meta: JSONObject?): String? = meta?.optJSONObject("sign_in")?.optString("device_id")?.ifBlank { null }

internal fun alertDialog(meta: JSONObject?): HqClient.AlertDialog? {
    val d = meta?.optJSONObject("dialog") ?: return null
    val session = d.optString("session_id", "").ifBlank { return null }
    if (!d.has("seq")) return null
    fun text(key: String) = if (d.isNull(key)) null else d.optString(key, "").ifBlank { null }
    val approve = d.optJSONObject("approve")?.let { a -> if (a.isNull("label")) null else a.optString("label", "").ifBlank { null } }
    return HqClient.AlertDialog(
        session, d.optLong("seq"), d.optString("kind", "other"), text("question"), text("command"), approve, d.optBoolean("answered", false),
        d.optBoolean("loses", false), losingVerb(d),
    )
}

internal val LOSING_VERBS = setOf("delete", "remove", "stop", "sign out", "revoke", "disconnect", "drop", "overwrite")

private fun losingVerb(o: JSONObject): String? = if (o.isNull("verb")) null else o.optString("verb", "").trim().lowercase().takeIf { it in LOSING_VERBS }

/** The approval in a notification's metadata, or null for any other notice. */
/** The count of times this site was allowed, from the item or its approval; null when HQ does not say. */
internal fun siteApprovals(item: JSONObject, meta: JSONObject?): Int? {
    val approval = meta?.optJSONObject("approval")
    return when {
        item.has("site_approvals") && !item.isNull("site_approvals") -> item.optInt("site_approvals")
        approval != null && approval.has("site_approvals") && !approval.isNull("site_approvals") -> approval.optInt("site_approvals")
        else -> null
    }
}

internal fun approvalFacts(meta: JSONObject?): HqClient.ApprovalFacts? {
    val a = meta?.optJSONObject("approval") ?: return null
    val args = if (a.isNull("args")) "" else a.opt("args")?.let { if (it is String) it else it.toString() }.orEmpty()
    return HqClient.ApprovalFacts(a.optString("tool", ""), args, a.optBoolean("loses", false), losingVerb(a), a.optString("thread_id", "").ifBlank { null }, if (a.isNull("site")) null else a.optString("site", "").ifBlank { null })
}

internal fun JSONArray?.objects(): List<JSONObject> = if (this == null) emptyList() else (0 until length()).map { getJSONObject(it) }
internal fun JSONArray?.strings(): List<String> = if (this == null) emptyList() else (0 until length()).map { getString(it) }

/** The helper list as the server sends it. A time the server has not got is JSON null, which is read as empty, never as the word "null". */
internal fun helperRows(text: String): List<HqClient.HelperRow> = JSONObject(text).optJSONArray("sessions").objects().map { o ->
    HqClient.HelperRow(
        o.getString("id"), o.optString("harness", ""), o.optString("label", ""), o.optString("host", ""), o.optString("cwd", ""),
        o.optString("status", ""), o.optString("agent_status", ""), o.optStringOrNull("last_seen_at").orEmpty(), o.optString("created_at", ""),
        o.optStringOrNull("goal"), o.optJSONObject("task")?.optStringOrNull("title"),
        if (o.isNull("alive") || !o.has("alive")) null else o.optBoolean("alive"),
        o.optBoolean("reachable", true), o.optStringOrNull("detail"), o.optBoolean("archived", false),
        helperDecision(o.optJSONObject("decision")),
    )
}

/** The open dialog of a helper, or null when absent or unreadable (the screen path then stays in use). */
internal fun helperDecision(d: JSONObject?): HqClient.HelperDecision? {
    val itemId = d?.optStringOrNull("item_id")?.ifBlank { null } ?: return null
    if (!d.has("seq") || d.isNull("seq")) return null
    // optJSONObject, not objects(): one odd option must not take the whole helper list down.
    val options = (0 until (d.optJSONArray("options")?.length() ?: 0)).mapNotNull { i ->
        val o = d.optJSONArray("options")?.optJSONObject(i) ?: return@mapNotNull null
        o.optStringOrNull("label")?.let { HqClient.HelperOption(o.optInt("id"), it, o.optBoolean("remember", false), o.optBoolean("typed", false)) }
    }
    return HqClient.HelperDecision(itemId, d.optLong("seq"), options, d.optStringOrNull("question"))
}
