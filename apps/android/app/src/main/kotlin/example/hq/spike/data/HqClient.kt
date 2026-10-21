package example.hq.spike.data

import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONArray
import org.json.JSONObject

/** The local hq is not answering, or answered with an error. */
class HqUnavailable(message: String, cause: Throwable? = null) : IOException(message, cause)

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

    suspend fun wsTicket(): String = send(authed("/api/ws-ticket").post(EMPTY_JSON).build()) { JSONObject(it).getString("ticket") }

    fun openSocket(ticket: String, listener: WebSocketListener): WebSocket =
        http.newWebSocket(Request.Builder().url("${base.replaceFirst("http", "ws")}/ws?ticket=$ticket").build(), listener)

    private fun authed(path: String) = Request.Builder().url(base + path).header("Authorization", "Bearer ${token()}")

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

    companion object {
        const val LOCAL_HQ = "http://127.0.0.1:5678"
        private const val HTTP_NOT_FOUND = 404
        private const val HTTP_BAD_REQUEST = 400
        private const val CONNECT_SECONDS = 3L
        private const val READ_SECONDS = 20L
        private const val PING_SECONDS = 20L
        private const val THREAD_PAGE = 50
        private const val MESSAGE_PAGE = 100
        private val EMPTY_JSON = "{}".toRequestBody("application/json".toMediaType())
    }
}

internal fun JSONArray?.objects(): List<JSONObject> = if (this == null) emptyList() else (0 until length()).map { getJSONObject(it) }
internal fun JSONArray?.strings(): List<String> = if (this == null) emptyList() else (0 until length()).map { getString(it) }
