package example.hq.spike.data

import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import example.hq.spike.ui.chat.blocks.Attachment
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONArray
import org.json.JSONObject

/** The local hq is not answering, or answered with an error. */
class HqUnavailable(message: String, cause: Throwable? = null) : IOException(message, cause)

/** hq cannot make that export from this note (no tables, a file that is not Markdown); [message] says why. */
class ExportRefused(message: String) : IOException(message)

/** An export saved in the app's cache. */
data class ExportedFile(val file: java.io.File, val name: String, val mime: String)

/** hq answered, and the note is not there (or the path is not one it will serve). */
class NoteNotFound(path: String) : IOException("No note at $path")

/** The same REST and web socket API the web app uses, over loopback. Auth is the sealed web token. */
class HqClient(private val token: () -> String, private val base: String = LOCAL_HQ) {
    /** Starting a helper answers only once the agent has settled, which can take longer than any other call here. */
    private val startHttp by lazy { http.newBuilder().readTimeout(START_READ_SECONDS, TimeUnit.SECONDS).build() }

    private val http = OkHttpClient.Builder()
        .connectTimeout(CONNECT_SECONDS, TimeUnit.SECONDS)
        .readTimeout(READ_SECONDS, TimeUnit.SECONDS)
        .pingInterval(PING_SECONDS, TimeUnit.SECONDS)
        .build()

    suspend fun healthy(): Boolean = withContext(Dispatchers.IO) {
        try {
            http.newCall(Request.Builder().url("$base/health").build()).execute().use { it.isSuccessful }
        } catch (e: IOException) {
            false
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

    suspend fun archive(threadId: String) {
        send(authed("/api/threads/$threadId/archive").post(EMPTY_JSON).build()) { }
    }

    /** One note as the web reader gets it. A 404 or a refused path is [NoteNotFound], not an outage. */
    suspend fun note(path: String): NoteBody = withContext(Dispatchers.IO) {
        val request = authed("/api/note?path=${java.net.URLEncoder.encode(path, "UTF-8")}").build()
        try {
            http.newCall(request).execute().use { r ->
                if (r.code == HTTP_NOT_FOUND || r.code == HTTP_BAD_REQUEST) throw NoteNotFound(path)
                if (!r.isSuccessful) throw HqUnavailable("HTTP ${r.code} for /api/note")
                val o = JSONObject(r.body.string())
                if (o.optBoolean("isDir", false)) throw NoteNotFound(path)
                NoteBody(o.optString("path", path), o.optString("content", ""), o.optBoolean("truncated", false))
            }
        } catch (e: IOException) {
            throw if (e is HqUnavailable || e is NoteNotFound) e else HqUnavailable("hq is not reachable", e)
        } catch (e: org.json.JSONException) {
            throw HqUnavailable("hq sent something unreadable", e)
        }
    }

    /** Saves a vault file (an upload, say) to [dest]. */
    suspend fun download(path: String, dest: java.io.File) = withContext(Dispatchers.IO) {
        val request = authed("/api/vault-asset?path=${java.net.URLEncoder.encode(path, "UTF-8")}").build()
        try {
            http.newCall(request).execute().use { r ->
                if (!r.isSuccessful) throw HqUnavailable("HTTP ${r.code} for /api/vault-asset")
                dest.parentFile?.mkdirs()
                r.body.byteStream().use { input -> dest.outputStream().use { input.copyTo(it) } }
            }
        } catch (e: IOException) {
            throw if (e is HqUnavailable) e else HqUnavailable("hq is not reachable", e)
        }
    }

    /** Makes one export of a note on hq and saves it under [dir]; cancelling the calling coroutine cancels the request. */
    suspend fun exportNote(path: String, format: String, dir: java.io.File): ExportedFile {
        val request = authed("/api/note/export?path=${java.net.URLEncoder.encode(path, "UTF-8")}&format=$format").build()
        val response = kotlinx.coroutines.suspendCancellableCoroutine { cont ->
            val call = http.newCall(request)
            cont.invokeOnCancellation { call.cancel() }
            call.enqueue(object : okhttp3.Callback {
                override fun onFailure(call: okhttp3.Call, e: IOException) = cont.resumeWith(Result.failure(HqUnavailable("hq is not reachable", e)))
                override fun onResponse(call: okhttp3.Call, response: okhttp3.Response) = cont.resumeWith(Result.success(response))
            })
        }
        return withContext(Dispatchers.IO) {
            response.use { r ->
                if (r.code == HTTP_BAD_REQUEST) throw ExportRefused(runCatching { JSONObject(r.body.string()).optString("error") }.getOrNull().orEmpty())
                if (!r.isSuccessful) throw HqUnavailable("HTTP ${r.code} for /api/note/export")
                val name = fileNameOf(r.header("Content-Disposition")) ?: "note.$format"
                val dest = java.io.File(dir, name)
                dest.parentFile?.mkdirs()
                r.body.byteStream().use { input -> dest.outputStream().use { input.copyTo(it) } }
                ExportedFile(dest, name, r.header("Content-Type")?.substringBefore(';')?.trim().orEmpty())
            }
        }
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
    )

    data class TaskComment(val id: String, val author: String, val body: String, val createdAt: String)

    private fun taskOf(o: JSONObject) = TaskItem(
        o.getString("id"), o.optString("display_id", ""), o.optString("title", ""), o.optString("description", ""), o.optString("status", "to_do"),
        o.optStringOrNull("priority"), o.optStringOrNull("due_date"), o.optStringOrNull("start_date"), o.optJSONArray("tags").strings(), o.optJSONArray("assignees").strings(),
        o.optStringOrNull("parent_task_id"), o.optInt("subtask_count", 0), o.optInt("subtask_done", 0), o.optStringOrNull("blocked_reason"),
        o.optStringOrNull("first_ready_for_review_at"), o.optString("updated_at", ""),
    )

    private fun json(body: JSONObject) = body.toString().toRequestBody("application/json".toMediaType())

    /** Active tasks, or the sub-tasks of one. The lists group and count them on the phone. */
    suspend fun tasks(parentId: String? = null, limit: Int = TASK_PAGE): List<TaskItem> =
        send(authed("/api/tasks?limit=$limit" + (parentId?.let { "&parent_task_id=$it" } ?: "&top_level_only=true")).build()) { text -> JSONObject(text).optJSONArray("tasks").objects().map(::taskOf) }

    /** Changes some fields of a task; a field left out is untouched, a JSON null clears it. 409 means it changed meanwhile. */
    suspend fun patchTask(id: String, change: JSONObject): TaskItem =
        send(authed("/api/tasks/${java.net.URLEncoder.encode(id, "UTF-8")}").patch(json(change)).build()) { taskOf(JSONObject(it)) }

    suspend fun createTask(body: JSONObject): TaskItem = send(authed("/api/tasks").post(json(body)).build()) { taskOf(JSONObject(it)) }

    suspend fun taskComments(id: String): List<TaskComment> =
        send(authed("/api/tasks/${java.net.URLEncoder.encode(id, "UTF-8")}/comments").build()) { text ->
            JSONObject(text).optJSONArray("comments").objects().map { TaskComment(it.optString("id"), it.optString("author", ""), it.optString("body", ""), it.optString("created_at", "")) }
        }

    suspend fun addTaskComment(id: String, body: String) {
        send(authed("/api/tasks/${java.net.URLEncoder.encode(id, "UTF-8")}/comments").post(json(JSONObject().put("body", body).put("author", "operator"))).build()) { }
    }

    suspend fun archiveTask(id: String) { send(authed("/api/tasks/${java.net.URLEncoder.encode(id, "UTF-8")}").delete().build()) { } }

    suspend fun restoreTask(id: String) { send(authed("/api/tasks/${java.net.URLEncoder.encode(id, "UTF-8")}/restore").post(json(JSONObject())).build()) { } }

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
    )

    data class InboxList(val items: List<InboxItem>, val unread: Int, val pending: Int)

    /** The inbox, newest first. [state] is pending, approved, dismissed, resolved or history; null for all. */
    suspend fun notifications(state: String?, limit: Int = 50): InboxList =
        send(authed("/api/notifications?limit=$limit" + (state?.let { "&state=$it" } ?: "")).build()) { text ->
            val o = JSONObject(text)
            InboxList(
                o.optJSONArray("notifications").objects().map { n ->
                    val meta = n.optJSONObject("metadata")
                    val extra = meta?.optJSONObject("extra")
                    InboxItem(
                        n.getString("id"), n.optString("kind", ""), n.optString("title", ""), n.optString("description", ""), n.optString("state", ""),
                        n.optString("created_at", ""), meta?.optStringOrNull("link"), extra?.optStringOrNull("source_task"), extra?.optStringOrNull("val_kind"),
                        n.optJSONArray("actions").strings(),
                    )
                },
                o.optInt("unread_count", 0), o.optInt("pending_approvals_count", 0),
            )
        }

    /** Records the owner's answer: approve, reject or dismiss. The thing that asked acts on it. */
    suspend fun answerNotification(id: String, action: String) {
        send(authed("/api/notifications/${java.net.URLEncoder.encode(id, "UTF-8")}/action").post(JSONObject().put("action", action).toString().toRequestBody("application/json".toMediaType())).build()) { }
    }

    /** Types keys such as enter or esc into a helper's terminal. 409 means it is no longer at a prompt. */
    suspend fun sendSessionKeys(sessionId: String, keys: List<String>) {
        send(authed("/api/harness-sessions/${java.net.URLEncoder.encode(sessionId, "UTF-8")}/send").post(JSONObject().put("keys", JSONArray(keys)).toString().toRequestBody("application/json".toMediaType())).build()) { }
    }

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
    )

    data class HelperHost(val name: String, val reachable: Boolean, val error: String?)

    /** Every helper, newest first, including stopped ones. */
    suspend fun helpers(): List<HelperRow> = send(authed("/api/harness-sessions").build()) { helperRows(it) }

    /** What a helper's terminal shows: its last lines, and whether they were read from the live terminal or are an old snapshot. */
    data class HelperScreen(val lines: List<String>, val live: Boolean)

    suspend fun helperScreen(id: String, lines: Int = SCREEN_LINES): HelperScreen =
        send(authed("/api/harness-sessions/${java.net.URLEncoder.encode(id, "UTF-8")}/screen?lines=$lines").build()) {
            val o = JSONObject(it)
            HelperScreen(o.optJSONArray("lines").strings(), o.optString("source", "") == "live")
        }

    /** Types a line into a helper. 409 means it is waiting at a prompt and must be answered there first. */
    suspend fun sendHelperText(id: String, text: String) {
        send(authed("/api/harness-sessions/${java.net.URLEncoder.encode(id, "UTF-8")}/send").post(JSONObject().put("text", text).toString().toRequestBody("application/json".toMediaType())).build()) { }
    }

    suspend fun stopHelper(id: String) {
        send(authed("/api/harness-sessions/${java.net.URLEncoder.encode(id, "UTF-8")}/stop").post(EMPTY_JSON).build()) { }
    }

    suspend fun renameHelper(id: String, label: String) {
        send(authed("/api/harness-sessions/${java.net.URLEncoder.encode(id, "UTF-8")}/rename").post(JSONObject().put("label", label).toString().toRequestBody("application/json".toMediaType())).build()) { }
    }

    /** Starts a helper in a new folder on [host], given its first instruction; returns the new session's id. */
    suspend fun startHelper(harness: String, host: String, prompt: String, label: String): String =
        send(
            authed("/api/harness-sessions").post(JSONObject().put("harness", harness).put("host", host).put("prompt", prompt).put("label", label).toString().toRequestBody("application/json".toMediaType())).build(),
            caller = startHttp,
        ) { JSONObject(it).getString("session_id") }

    /** The computers helpers can run on, and whether each answered. */
    suspend fun helperHosts(): List<HelperHost> = send(authed("/api/workbench/hosts").build()) { text ->
        JSONObject(text).optJSONArray("hosts").objects().mapNotNull { h -> h.optStringOrNull("host")?.let { HelperHost(it, h.optBoolean("reachable", false), h.optStringOrNull("error")) } }
    }

    /** A read-only JSON route as it is, for the Settings pages that only show what the server says. */
    suspend fun json(path: String): JSONObject = send(authed(path).build()) { JSONObject(it) }

    /** Notes in a list: path, title, a short preview, when it changed and whether it is pinned. */
    data class NoteItem(val path: String, val title: String, val preview: String, val mtime: String, val pinned: Boolean)

    private fun noteItems(array: JSONArray?) = array.objects().map {
        NoteItem(it.getString("path"), it.optString("title", ""), it.optString("preview", ""), it.optString("mtime", ""), it.optBoolean("pinned", false))
    }

    /** The newest notes, as the home screen's Recent list. */
    suspend fun recentNotes(): List<NoteItem> = send(authed("/api/vault-signals").build()) { noteItems(JSONObject(it).optJSONArray("recent")) }

    suspend fun pinnedNotes(): List<NoteItem> = send(authed("/api/pinned").build()) { noteItems(JSONObject(it).optJSONArray("notes")) }

    /** Every folder under Notebooks, Inbox first, as vault paths. */
    suspend fun folders(): List<String> = send(authed("/api/vault/folders").build()) { JSONObject(it).optJSONArray("folders").strings() }

    /** One folder's contents: sub-folders with the notes under each, and the notes directly inside. */
    data class FolderListing(val folders: List<Pair<String, Int>>, val notes: List<String>)

    suspend fun folderListing(path: String): FolderListing =
        send(authed("/api/tree?recursive=true&path=${java.net.URLEncoder.encode(path, "UTF-8")}").build()) { text ->
            val tree = JSONObject(text).optJSONObject("tree")
            val children = tree?.optJSONArray("children").objects()
            fun count(o: JSONObject): Int = if (o.optString("type") == "file") 1 else o.optJSONArray("children").objects().sumOf(::count)
            FolderListing(
                children.filter { it.optString("type") == "dir" }.map { it.optString("path") to count(it) },
                children.filter { it.optString("type") == "file" && it.optString("name").endsWith(".md") }.map { it.optString("path") },
            )
        }

    /** Full-text matches with a snippet of the text around the match. */
    data class NoteHit(val path: String, val title: String, val notebook: String, val snippet: String)

    suspend fun searchNoteHits(query: String, limit: Int = 30): List<NoteHit> =
        send(authed("/api/search?q=${java.net.URLEncoder.encode(query, "UTF-8")}&limit=$limit").build()) { text ->
            JSONObject(text).optJSONArray("results").objects().map {
                NoteHit(it.getString("note_path"), it.optString("title", ""), it.optString("notebook", ""), it.optString("snippet", ""))
            }
        }

    /** Makes a new note and returns its vault path; the title only names the file. */
    suspend fun createNote(folder: String, title: String, content: String): String =
        send(authed("/api/note/create").post(JSONObject().put("folder", folder).put("title", title).put("content", content).toString().toRequestBody("application/json".toMediaType())).build()) {
            JSONObject(it).getString("path")
        }

    /** Replaces a note's text (body only: the server keeps the front matter). Last write wins; there is no version check yet. */
    suspend fun saveNote(path: String, content: String) {
        send(authed("/api/note").put(JSONObject().put("path", path).put("content", content).toString().toRequestBody("application/json".toMediaType())).build()) { }
    }

    /** The vault paths of the pinned notes. */
    suspend fun pinned(): Set<String> =
        send(authed("/api/pinned").build()) { text -> JSONObject(text).optJSONArray("notes").objects().mapNotNull { it.optString("path").ifBlank { null } }.toSet() }

    suspend fun setPinned(path: String, pin: Boolean) {
        send(authed("/api/pin").post(JSONObject().put("path", path).put("pin", pin).toString().toRequestBody("application/json".toMediaType())).build()) { }
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

    /** Notes whose text matches, best first: (vault path, title, notebook). */
    suspend fun searchNotes(query: String, limit: Int = PICKER_PAGE): List<Triple<String, String, String>> =
        send(authed("/api/search?q=${java.net.URLEncoder.encode(query, "UTF-8")}&limit=$limit").build()) { text ->
            JSONObject(text).optJSONArray("results").objects().map { Triple(it.getString("note_path"), it.optString("title", ""), it.optString("notebook", "")) }
        }

    /** Top-level tasks, newest first, optionally filtered by words: (display id, title, status). */
    suspend fun searchTasks(query: String, limit: Int = PICKER_PAGE): List<Triple<String, String, String>> =
        send(authed("/api/tasks?top_level_only=true&limit=$limit&search=${java.net.URLEncoder.encode(query, "UTF-8")}").build()) { text ->
            JSONObject(text).optJSONArray("tasks").objects().map { Triple(it.getString("display_id"), it.optString("title", ""), it.optString("status", "")) }
        }

    suspend fun wsTicket(): String = send(authed("/api/ws-ticket").post(EMPTY_JSON).build()) { JSONObject(it).getString("ticket") }

    fun openSocket(ticket: String, listener: WebSocketListener): WebSocket =
        http.newWebSocket(Request.Builder().url("${base.replaceFirst("http", "ws")}/ws?ticket=$ticket").build(), listener)

    /** The server wants this header on requests that change a helper, setup or a credential; sending it on every request is harmless. */
    private fun authed(path: String) = Request.Builder().url(base + path).header("Authorization", "Bearer ${token()}").header("X-HQ-Client", "android")

    private suspend fun <T> send(request: Request, caller: OkHttpClient = http, parse: (String) -> T): T = withContext(Dispatchers.IO) {
        try {
            caller.newCall(request).execute().use { r ->
                if (!r.isSuccessful) throw HqUnavailable("HTTP ${r.code} for ${request.url.encodedPath}")
                parse(r.body.string())
            }
        } catch (e: IOException) {
            throw if (e is HqUnavailable) e else HqUnavailable("hq is not reachable", e)
        } catch (e: org.json.JSONException) {
            throw HqUnavailable("hq sent something unreadable", e)
        }
    }

    /** The file name a download asks for: the UTF-8 form when there is one, else the plain one; never a path. */
    private fun fileNameOf(disposition: String?): String? {
        val d = disposition ?: return null
        val raw = Regex("filename\\*=UTF-8''([^;]+)").find(d)?.groupValues?.get(1)?.let { java.net.URLDecoder.decode(it, "UTF-8") }
            ?: Regex("filename=\"([^\"]+)\"").find(d)?.groupValues?.get(1)
        return raw?.substringAfterLast('/')?.replace('\\', '_')?.takeIf { it.isNotBlank() && it != ".." }
    }

    companion object {
        const val LOCAL_HQ = "http://127.0.0.1:5678"
        private const val HTTP_NOT_FOUND = 404
        private const val HTTP_BAD_REQUEST = 400
        private const val CONNECT_SECONDS = 3L
        private const val READ_SECONDS = 20L
        private const val START_READ_SECONDS = 150L
        private const val PING_SECONDS = 20L
        private const val THREAD_PAGE = 50
        private const val PICKER_PAGE = 20
        private const val TASK_PAGE = 500
        private const val SCREEN_LINES = 80
        private const val UPLOAD_CHUNK = 64 * 1024
        const val MESSAGE_PAGE = 100
        private val EMPTY_JSON = "{}".toRequestBody("application/json".toMediaType())
    }
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
    )
}
