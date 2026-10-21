package example.hq.spike.ui.chat

import android.app.Application
import example.hq.spike.data.ExportRefused
import example.hq.spike.data.HqClient
import example.hq.spike.data.HqUnavailable
import example.hq.spike.ui.chat.blocks.Block
import example.hq.spike.ui.chat.blocks.NoteText
import example.hq.spike.ui.chat.blocks.parseBlocks
import java.io.File
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** Makes an export of an open note on hq: pick a format, wait with a way out, then hold the file for Share, Save or Open. */
class NoteExporter(
    private val app: Application,
    private val client: HqClient,
    private val state: MutableStateFlow<ChatUiState>,
    private val scope: CoroutineScope,
) {
    private var job: Job? = null

    fun choose(sheet: NoteSheet) {
        val tables = parseBlocks(NoteText.withoutFrontmatter(sheet.body)).count { it is Block.Table }
        state.update { it.copy(noteMenu = false, export = ExportState.Choosing(ExportNote(sheet.path, sheet.title, tables))) }
    }

    fun start(format: ExportFormat) {
        val note = state.value.export?.note ?: return
        job?.cancel()
        state.update { it.copy(export = ExportState.Making(note, format)) }
        job = scope.launch {
            val next = try {
                val done = client.exportNote(note.path, format.key, File(app.cacheDir, EXPORT_DIR))
                ExportState.Ready(note, format, done.name, done.mime, done.file.absolutePath, done.file.length())
            } catch (e: CancellationException) {
                throw e
            } catch (e: ExportRefused) {
                ExportState.Failed(note, format, e.message?.takeIf { it.isNotBlank() } ?: failed(format))
            } catch (e: HqUnavailable) {
                ExportState.Failed(note, format, "HQ is not running")
            }
            state.update { s -> if (s.export is ExportState.Making) s.copy(export = next) else s }
        }
    }

    /** Cancel while it is being made, or Close on any other step: back to the note. */
    fun close() {
        job?.cancel()
        state.update { it.copy(export = null) }
    }

    /** Back from a failure to the list of formats. */
    fun again() = state.update { s -> s.export?.let { s.copy(export = ExportState.Choosing(it.note)) } ?: s }

    private fun failed(format: ExportFormat) = "Could not make the ${format.word}. Try again."

    companion object {
        const val EXPORT_DIR = "exports"
    }
}
