package example.hq.spike.ui.settings

import android.content.Context
import android.net.Uri
import android.provider.DocumentsContract
import example.hq.spike.HubService
import example.hq.spike.R
import example.hq.spike.safe.NotesCopy
import java.io.IOException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

/**
 * Save a copy now: the same zip the safe-mode screen makes, written to a place the person picks. It lives here, not in a screen,
 * so leaving the page does not stop it. One copy at a time.
 */
object BackupCopy {
    sealed interface State {
        data object Idle : State

        /** [percent] is 0 to 100; the sizes are in megabytes. */
        data class Busy(val percent: Int, val doneMb: Long, val totalMb: Long) : State

        data class Done(val atMs: Long, val onPhone: Boolean) : State

        data class Failed(val message: String) : State
    }

    private const val MB = 1_000_000L
    private const val PERCENT = 100
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val _state = MutableStateFlow<State>(State.Idle)
    val state: StateFlow<State> = _state

    @Volatile private var cancel = false
    private var job: Job? = null

    /** Back to the plain card, after Done or Failed has been seen. */
    fun clear() { if (_state.value !is State.Busy) _state.value = State.Idle }

    /** Stops a copy under way. Nothing is said: the unfinished file is removed and the card goes back as it was. */
    fun cancel() { cancel = true }

    /** Writes the copy to [uri], a document the person just created. Does nothing while another copy runs. */
    fun start(context: Context, uri: Uri) {
        val app = context.applicationContext
        if (_state.value is State.Busy) return
        cancel = false
        _state.value = State.Busy(0, 0, 0)
        job = scope.launch { run(app, uri) }
    }

    private fun run(app: Context, uri: Uri) {
        val files = app.filesDir
        var held = false
        try {
            val plan = NotesCopy.plan(files)
            val free = freeBytes(app, uri)
            if (NotesCopy.short(plan, free)) {
                runCatching { DocumentsContract.deleteDocument(app.contentResolver, uri) }
                val onPhone = onPhone(uri)
                _state.value = State.Failed(
                    if (onPhone) app.getString(R.string.safe_fail_phone_full, NotesCopy.size(NotesCopy.freeUpBytes(plan, free ?: 0)))
                    else app.getString(R.string.safe_fail_place_full),
                )
                return
            }
            NotesCopy.markPending(files, uri.toString())
            if (plan.databaseFiles > 0) held = HubService.holdHq()
            val out = app.contentResolver.openOutputStream(uri, "w") ?: throw IOException("No output")
            out.buffered(1 shl 16).use { stream ->
                NotesCopy.write(
                    plan, stream, cancelled = { cancel },
                    onDatabaseCopied = { if (held) { HubService.releaseHq(); held = false } },
                ) { p ->
                    val pct = if (p.bytesTotal > 0) (p.bytesDone * PERCENT / p.bytesTotal).toInt().coerceIn(0, PERCENT) else 0
                    _state.value = State.Busy(pct, p.bytesDone / MB, p.bytesTotal / MB)
                }
            }
            NotesCopy.clearPending(files)
            val onPhone = onPhone(uri)
            val now = System.currentTimeMillis()
            NotesCopy.recordCopy(files, now, onPhone)
            _state.value = State.Done(now, onPhone)
        } catch (e: NotesCopy.Cancelled) {
            discard(app, uri)
            _state.value = State.Idle
        } catch (e: Exception) {
            discard(app, uri)
            _state.value = State.Failed(
                app.getString(if (NotesCopy.classify(e) == NotesCopy.Failure.PlaceFull) R.string.safe_fail_place_full else R.string.safe_fail_place_gone),
            )
        } finally {
            if (held) HubService.releaseHq()
        }
    }

    private fun discard(app: Context, uri: Uri) {
        runCatching { DocumentsContract.deleteDocument(app.contentResolver, uri) }
        NotesCopy.clearPending(app.filesDir)
    }

    private fun onPhone(uri: Uri) = NotesCopy.onThisPhone(uri.authority, runCatching { DocumentsContract.getDocumentId(uri) }.getOrNull())

    /** The bytes the provider says are free where the document is, or null when it does not say. */
    private fun freeBytes(app: Context, uri: Uri): Long? = try {
        val docId = DocumentsContract.getDocumentId(uri)
        var best: Long? = null
        var bestLen = -1
        app.contentResolver.query(DocumentsContract.buildRootsUri(uri.authority), null, null, null, null)?.use { c ->
            val doc = c.getColumnIndex(DocumentsContract.Root.COLUMN_DOCUMENT_ID)
            val avail = c.getColumnIndex(DocumentsContract.Root.COLUMN_AVAILABLE_BYTES)
            while (c.moveToNext()) {
                val rootDoc = if (doc >= 0) c.getString(doc).orEmpty() else ""
                if (rootDoc.isNotEmpty() && docId.startsWith(rootDoc.substringBefore(':')) && rootDoc.length > bestLen) {
                    bestLen = rootDoc.length
                    best = if (avail >= 0 && !c.isNull(avail)) c.getLong(avail) else null
                }
            }
        }
        best
    } catch (_: Exception) {
        null
    }
}
