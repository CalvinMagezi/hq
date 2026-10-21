package example.hq.spike.ui.notes

import androidx.compose.ui.text.input.TextFieldValue
import example.hq.spike.data.HqClient.NoteItem
import example.hq.spike.ui.chat.ExportState
import example.hq.spike.ui.chat.Notice
import example.hq.spike.ui.chat.NoteSheet

/** A page pushed over the Notes home. Back pops one. */
sealed interface NotesRoute {
    data class Folder(val path: String) : NotesRoute
    data object Search : NotesRoute
    data class Reader(val path: String) : NotesRoute
    data class Write(val path: String?) : NotesRoute
}

/** What the reader is showing for a note: still fetching, ready, gone (a 404), or HQ not answering. */
sealed interface ReaderPhase {
    data object Loading : ReaderPhase
    data class Ready(val note: NoteSheet, val edited: String) : ReaderPhase
    data object Gone : ReaderPhase
    data object Failed : ReaderPhase
}

data class FolderView(val path: String, val folders: List<Pair<String, Int>> = emptyList(), val notes: List<String> = emptyList(), val loading: Boolean = true)

data class SearchHit(val path: String, val title: String, val notebook: String, val snippet: String)

data class SearchState(val query: String = "", val hits: List<SearchHit> = emptyList(), val filter: String? = null, val loading: Boolean = false)

enum class SaveStatus { Saved, Unsaved, Saving, Failed }

data class WriteState(
    val path: String?,
    val folder: String,
    val title: String,
    val text: TextFieldValue,
    val original: String,
    val preview: Boolean = false,
    val status: SaveStatus = SaveStatus.Saved,
    val chooser: Boolean = false,
    val leaving: Boolean = false,
) {
    val isNew: Boolean get() = path == null
    val dirty: Boolean get() = text.text != original || (isNew && title.isNotBlank())
}

data class NotesUi(
    val loaded: Boolean = false,
    val offline: Boolean = false,
    val recent: List<NoteItem> = emptyList(),
    val pinned: List<NoteItem> = emptyList(),
    val folders: List<String> = emptyList(),
    val stack: List<NotesRoute> = emptyList(),
    val folder: FolderView? = null,
    val search: SearchState = SearchState(),
    val reader: ReaderPhase = ReaderPhase.Loading,
    val write: WriteState? = null,
    val menu: Boolean = false,
    val export: ExportState? = null,
    val notice: Notice? = null,
) {
    val top: NotesRoute? get() = stack.lastOrNull()

    /** The folders directly under Notebooks, Inbox first, for the chips on the home screen. */
    val topFolders: List<String> get() = folders.map { it.removePrefix("Notebooks/").substringBefore('/') }.filter { it.isNotBlank() }.distinct()
}
