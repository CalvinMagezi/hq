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

    private suspend fun <T> send(request: Request, parse: (String) -> T): T = withContext(Dispatchers.IO) {
        try {
            http.newCall(request).execute().use { r ->
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
        private const val PING_SECONDS = 20L
        private const val THREAD_PAGE = 50
        private const val PICKER_PAGE = 20
        private const val UPLOAD_CHUNK = 64 * 1024
        const val MESSAGE_PAGE = 100
        private val EMPTY_JSON = "{}".toRequestBody("application/json".toMediaType())
    }
}

internal fun JSONArray?.objects(): List<JSONObject> = if (this == null) emptyList() else (0 until length()).map { getJSONObject(it) }
internal fun JSONArray?.strings(): List<String> = if (this == null) emptyList() else (0 until length()).map { getString(it) }
