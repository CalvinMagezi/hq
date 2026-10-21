package example.hq.spike.ui.chat

import android.app.Application
import android.net.Uri
import android.provider.OpenableColumns
import example.hq.spike.data.HqClient
import example.hq.spike.data.HqUnavailable
import example.hq.spike.ui.chat.blocks.NoteRefs
import java.util.UUID
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** What sits around the message field: the plus menu, the note and task pickers, attached items, and files going up. */
class ComposerController(
    private val app: Application,
    private val client: HqClient,
    private val state: MutableStateFlow<ChatUiState>,
    private val scope: CoroutineScope,
) {
    private var search: Job? = null

    fun openAdd() = state.update { it.copy(addOpen = true) }

    fun closeAdd() = state.update { it.copy(addOpen = false) }

    fun openPicker(kind: ContextRef.Kind) {
        state.update { it.copy(addOpen = false, picker = Picker(kind)) }
        runSearch(kind, "", wait = false)
    }

    fun closePicker() {
        search?.cancel()
        state.update { it.copy(picker = null) }
    }

    fun queryPicker(query: String) {
        val kind = state.value.picker?.kind ?: return
        state.update { s -> s.copy(picker = s.picker?.copy(query = query, loading = true)) }
        runSearch(kind, query, wait = true)
    }

    private fun runSearch(kind: ContextRef.Kind, query: String, wait: Boolean) {
        search?.cancel()
        search = scope.launch {
            if (wait) delay(SEARCH_WAIT_MS)
            val found = try {
                if (kind == ContextRef.Kind.Note) {
                    if (query.isBlank()) emptyList() else client.searchNotes(query).map { (path, title, _) -> ContextRef(kind, path, title.ifBlank { NoteRefs.title(path) }) }
                } else {
                    client.searchTasks(query).map { (id, title, _) -> ContextRef(kind, id, title) }
                }
            } catch (e: HqUnavailable) {
                emptyList()
            }
            state.update { s -> s.copy(picker = s.picker?.takeIf { it.kind == kind && it.query == query }?.copy(results = found, loading = false) ?: s.picker) }
        }
    }

    fun choose(item: ContextRef) {
        search?.cancel()
        state.update { s -> s.copy(picker = null, contexts = if (item in s.contexts) s.contexts else (s.contexts + item).takeLast(MAX_CONTEXT)) }
    }

    fun removeContext(item: ContextRef) = state.update { it.copy(contexts = it.contexts - item) }

    /** Takes the files the system picker returned: too big or empty ones become error chips, the rest start uploading. */
    fun addFiles(uris: List<Uri>) {
        for (uri in uris) {
            if (state.value.files.size >= MAX_FILES) {
                state.update { it.copy(notice = Notice("You can attach up to $MAX_FILES files to a message.")) }
                return
            }
            val (name, size) = describe(uri)
            val reject = when { size == 0L -> "empty file"; size > MAX_BYTES -> "over 25 MB"; else -> null }
            val file = PendingFile(
                UUID.randomUUID().toString(), uri.toString(), name, size, app.contentResolver.getType(uri).orEmpty(),
                if (reject == null) FileStatus.Uploading else FileStatus.Failed, reject,
            )
            state.update { it.copy(files = it.files + file) }
            if (reject == null) upload(file.id)
        }
    }

    fun retry(id: String) {
        update(id) { it.copy(status = FileStatus.Uploading, error = null) }
        upload(id)
    }

    fun removeFile(id: String) = state.update { s -> s.copy(files = s.files.filter { it.id != id }) }

    private fun upload(id: String) {
        val file = state.value.files.firstOrNull { it.id == id } ?: return
        scope.launch {
            val result = withContext(Dispatchers.IO) {
                try {
                    val bytes = app.contentResolver.openInputStream(Uri.parse(file.uri))?.use { it.readBytes() }
                    when {
                        bytes == null -> Result.failure(HqUnavailable("The file could not be read"))
                        bytes.size > MAX_BYTES -> Result.failure(TooBig())
                        else -> Result.success(client.uploadFile(file.name, file.mime, bytes))
                    }
                } catch (e: java.io.IOException) {
                    Result.failure(e)
                }
            }
            result.onSuccess { saved -> update(id) { it.copy(status = FileStatus.Ready, uploaded = saved, error = null) } }
                .onFailure { e -> update(id) { it.copy(status = FileStatus.Failed, error = if (e is TooBig) "over 25 MB" else PendingFile.UPLOAD_FAILED) } }
        }
    }

    private class TooBig : java.io.IOException()

    private fun update(id: String, change: (PendingFile) -> PendingFile) =
        state.update { s -> s.copy(files = s.files.map { if (it.id == id) change(it) else it }) }

    private fun describe(uri: Uri): Pair<String, Long> {
        app.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
            if (c.moveToFirst()) return (c.getString(0) ?: "file") to (if (c.isNull(1)) -1L else c.getLong(1))
        }
        return (uri.lastPathSegment ?: "file") to -1L
    }

    companion object {
        const val MAX_FILES = 10
        const val MAX_BYTES = 25 * 1024 * 1024
        private const val MAX_CONTEXT = 10
        private const val SEARCH_WAIT_MS = 250L
    }
}
