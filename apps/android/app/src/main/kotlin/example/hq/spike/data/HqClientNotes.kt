package example.hq.spike.data

import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject

// The note and folder calls of [HqClient]: reading, searching, saving, exporting, pins and the trash.

/** One note as the web reader gets it. A 404 or a refused path is [NoteNotFound], not an outage. */
suspend fun HqClient.note(path: String): NoteBody = withContext(Dispatchers.IO) {
    val request = authed("/api/note?path=${java.net.URLEncoder.encode(path, "UTF-8")}").build()
    try {
        http.newCall(request).execute().use { r ->
            if (r.code == HqClient.HTTP_UNAUTHORIZED) refused()
            if (r.code == HqClient.HTTP_NOT_FOUND || r.code == HqClient.HTTP_BAD_REQUEST) throw NoteNotFound(path)
            if (!r.isSuccessful) throw HqUnavailable("HTTP ${r.code} for /api/note", status = r.code)
            val o = JSONObject(r.body.string())
            if (o.optBoolean("isDir", false)) throw NoteNotFound(path)
            NoteWire.body(o, path)
        }
    } catch (e: IOException) {
        throw if (e is HqUnavailable || e is NoteNotFound) e else HqUnavailable("hq is not reachable", e)
    } catch (e: org.json.JSONException) {
        throw HqUnavailable("hq sent something unreadable", e, status = HqUnavailable.UNREADABLE)
    }
}

/** Where the note with this id lives now, for a `hq://notes/n/{id}` link. A 404 is [NoteNotFound]. */
suspend fun HqClient.notePathById(id: String): String = withContext(Dispatchers.IO) {
    val request = authed("/api/note/by-id/${java.net.URLEncoder.encode(id, "UTF-8")}").build()
    try {
        http.newCall(request).execute().use { r ->
            if (r.code == HqClient.HTTP_UNAUTHORIZED) refused()
            if (r.code == HqClient.HTTP_NOT_FOUND) throw NoteNotFound(id)
            if (!r.isSuccessful) throw HqUnavailable("HTTP ${r.code} for /api/note/by-id", status = r.code)
            JSONObject(r.body.string()).optString("path", "").ifBlank { throw NoteNotFound(id) }
        }
    } catch (e: IOException) {
        throw if (e is HqUnavailable || e is NoteNotFound) e else HqUnavailable("hq is not reachable", e)
    } catch (e: org.json.JSONException) {
        throw HqUnavailable("hq sent something unreadable", e, status = HqUnavailable.UNREADABLE)
    }
}

/** Saves a vault file (an upload, say) to [dest]. */
suspend fun HqClient.download(path: String, dest: java.io.File) = withContext(Dispatchers.IO) {
    val request = authed("/api/vault-asset?path=${java.net.URLEncoder.encode(path, "UTF-8")}").build()
    try {
        http.newCall(request).execute().use { r ->
            if (r.code == HqClient.HTTP_UNAUTHORIZED) refused()
            if (!r.isSuccessful) throw HqUnavailable("HTTP ${r.code} for /api/vault-asset", status = r.code)
            dest.parentFile?.mkdirs()
            r.body.byteStream().use { input -> dest.outputStream().use { input.copyTo(it) } }
        }
    } catch (e: IOException) {
        throw if (e is HqUnavailable) e else HqUnavailable("hq is not reachable", e)
    }
}

/** Makes one export of a note on hq and saves it under [dir]; cancelling the calling coroutine cancels the request. */
suspend fun HqClient.exportNote(path: String, format: String, dir: java.io.File): ExportedFile {
    val request = authed("/api/note/export?path=${java.net.URLEncoder.encode(path, "UTF-8")}&format=$format").build()
    val response = kotlinx.coroutines.suspendCancellableCoroutine { cont ->
        val call = startHttp.newCall(request)
        cont.invokeOnCancellation { call.cancel() }
        call.enqueue(object : okhttp3.Callback {
            override fun onFailure(call: okhttp3.Call, e: IOException) = cont.resumeWith(Result.failure(HqUnavailable("hq is not reachable", e)))
            override fun onResponse(call: okhttp3.Call, response: okhttp3.Response) = cont.resumeWith(Result.success(response))
        })
    }
    return withContext(Dispatchers.IO) {
        response.use { r ->
            if (r.code == HqClient.HTTP_BAD_REQUEST) throw ExportRefused(runCatching { JSONObject(r.body.string()).optString("error") }.getOrNull().orEmpty())
            if (r.code == HqClient.HTTP_UNAUTHORIZED) refused()
            if (!r.isSuccessful) throw HqUnavailable("HTTP ${r.code} for /api/note/export", status = r.code)
            val name = fileNameOf(r.header("Content-Disposition")) ?: "note.$format"
            val dest = java.io.File(dir, name)
            try {
                dest.parentFile?.mkdirs()
                r.body.byteStream().use { input -> dest.outputStream().use { input.copyTo(it) } }
            } catch (e: IOException) {
                dest.delete()
                throw HqUnavailable("the export could not be saved", e)
            }
            ExportedFile(dest, name, r.header("Content-Type")?.substringBefore(';')?.trim().orEmpty())
        }
    }
}

suspend fun HqClient.vaultSignals(): HqClient.VaultSignals = send(authed("/api/vault-signals").build()) {
    val o = JSONObject(it)
    HqClient.VaultSignals(noteItems(o.optJSONArray("recent")), o.optBoolean("has_new", false))
}

/** The owner opened this note: HQ stops calling it new. An older HQ has no such route, which is fine. */
suspend fun HqClient.markNoteSeen(path: String) {
    try { send(authed("/api/note/seen").post(json(JSONObject().put("path", path))).build()) { } } catch (e: HqUnavailable) { if (e.status != HqClient.HTTP_NOT_FOUND) throw e }
}

/** Keep: the owner accepts what HQ wrote. */
suspend fun HqClient.keepNote(path: String) {
    send(authed("/api/note/review").post(json(JSONObject().put("path", path).put("verdict", "accepted"))).build()) { }
}

/** The newest notes, as the home screen's Recent list. */
suspend fun HqClient.recentNotes(): List<HqClient.NoteItem> = send(authed("/api/vault-signals").build()) { noteItems(JSONObject(it).optJSONArray("recent")) }

suspend fun HqClient.pinnedNotes(): List<HqClient.NoteItem> = send(authed("/api/pinned").build()) { noteItems(JSONObject(it).optJSONArray("notes")) }

/** Every folder under Notebooks, Inbox first, as vault paths. */
suspend fun HqClient.folders(): List<String> = send(authed("/api/vault/folders").build()) { JSONObject(it).optJSONArray("folders").strings() }

suspend fun HqClient.folderListing(path: String): HqClient.FolderListing =
    send(authed("/api/tree?recursive=true&path=${java.net.URLEncoder.encode(path, "UTF-8")}").build()) { text ->
        val tree = JSONObject(text).optJSONObject("tree")
        val children = tree?.optJSONArray("children").objects()
        fun count(o: JSONObject): Int = if (o.optString("type") == "file") 1 else o.optJSONArray("children").objects().sumOf(::count)
        HqClient.FolderListing(
            children.filter { it.optString("type") == "dir" }.map { it.optString("path") to count(it) },
            children.filter { it.optString("type") == "file" && it.optString("name").endsWith(".md") }.map { it.optString("path") },
        )
    }

/** [folder] narrows the search to that notes folder and the ones inside it; HQ filters before it cuts to [limit]. */
suspend fun HqClient.searchNoteHits(query: String, limit: Int = 30, folder: String? = null, writtenByHq: Boolean = false, tag: String? = null): List<HqClient.NoteHit> =
    send(authed("/api/search?" + noteSearchQuery(query, limit, folder, writtenByHq, tag)).build()) { text ->
        JSONObject(text).optJSONArray("results").objects().map {
            HqClient.NoteHit(it.getString("note_path"), it.optString("title", ""), it.optString("notebook", ""), it.optString("snippet", ""))
        }
    }

/** Makes a new note and returns its vault path; the title only names the file. */
suspend fun HqClient.createNote(folder: String, title: String, content: String, clientId: String? = null, enrichUrl: String? = null): String =
    send(authed("/api/note/create").post(JSONObject().put("folder", folder).put("title", title).put("content", content).apply { clientId?.let { put("client_id", it) }; enrichUrl?.let { put("enrich_url", it) } }.toString().toRequestBody("application/json".toMediaType())).build()) {
        JSONObject(it).getString("path")
    }

suspend fun HqClient.deleteNote(path: String): String =
    send(authed("/api/note?path=${java.net.URLEncoder.encode(path, "UTF-8")}").delete().build()) {
        JSONObject(it).getString("trash_path")
    }

suspend fun HqClient.trash(): Pair<List<HqClient.TrashedNote>, Int> =
    send(authed("/api/note/trash").build()) { text ->
        val json = JSONObject(text)
        val items = json.optJSONArray("items").objects().map {
            HqClient.TrashedNote(it.getString("trash_path"), it.getString("original_path"), it.getString("trashed_on"), it.optInt("days_left", 0), it.optString("deleted_by", "") == WRITTEN_BY_HQ)
        }
        items to json.optInt("retention_days", 30)
    }

suspend fun HqClient.trashedText(trashPath: String): String =
    send(authed("/api/note/trash/read?path=${java.net.URLEncoder.encode(trashPath, "UTF-8")}").build()) {
        JSONObject(it).getString("content")
    }

suspend fun HqClient.restoreNote(trashPath: String): String = restoreNoteTo(trashPath).path

suspend fun HqClient.restoreNoteTo(trashPath: String): HqClient.Restored =
    send(authed("/api/note/trash/restore").post(JSONObject().put("trash_path", trashPath).toString().toRequestBody("application/json".toMediaType())).build()) {
        val o = JSONObject(it)
        HqClient.Restored(o.getString("path"), NoteWire.restoredTo(o))
    }

/** Replaces a note's text (body only: the server keeps the front matter). Last write wins; there is no version check yet. */
suspend fun HqClient.saveNote(path: String, content: String, base: String? = null) {
    val body = JSONObject().put("path", path).put("content", content).apply { base?.let { put("base", it) } }
    val request = authed("/api/note").put(body.toString().toRequestBody("application/json".toMediaType())).build()
    withContext(Dispatchers.IO) {
        try {
            http.newCall(request).execute().use { r ->
                if (r.code == HqClient.HTTP_UNAUTHORIZED) refused()
                if (r.code == HqClient.HTTP_CONFLICT) {
                    val o = runCatching { JSONObject(r.body.string()) }.getOrDefault(JSONObject())
                    throw NoteChanged(o.optString("theirs_hash", "").ifBlank { null }, o.optString("by", "").ifBlank { null }, o.optString("at", "").ifBlank { null })
                }
                if (!r.isSuccessful) throw HqUnavailable("HTTP ${r.code} for /api/note", status = r.code)
            }
        } catch (e: IOException) {
            throw if (e is HqUnavailable || e is NoteChanged) e else HqUnavailable("hq is not reachable", e)
        }
    }
}

/** The vault paths of the pinned notes. */
suspend fun HqClient.pinned(): Set<String> =
    send(authed("/api/pinned").build()) { text -> JSONObject(text).optJSONArray("notes").objects().mapNotNull { it.optString("path").ifBlank { null } }.toSet() }

suspend fun HqClient.setPinned(path: String, pin: Boolean) {
    send(authed("/api/pin").post(JSONObject().put("path", path).put("pin", pin).toString().toRequestBody("application/json".toMediaType())).build()) { }
}

/** Notes whose text matches, best first: (vault path, title, notebook). */
suspend fun HqClient.searchNotes(query: String, limit: Int = HqClient.PICKER_PAGE): List<Triple<String, String, String>> =
    send(authed("/api/search?q=${java.net.URLEncoder.encode(query, "UTF-8")}&limit=$limit").build()) { text ->
        JSONObject(text).optJSONArray("results").objects().map { Triple(it.getString("note_path"), it.optString("title", ""), it.optString("notebook", "")) }
    }
