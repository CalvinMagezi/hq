package example.hq.spike.ui.notes

import androidx.compose.ui.text.input.TextFieldValue
import example.hq.spike.data.HqClient
import example.hq.spike.data.HqClient.NoteItem
import example.hq.spike.ui.chat.ExportState
import example.hq.spike.ui.chat.Notice
import example.hq.spike.ui.chat.NoteSheet
import example.hq.spike.ui.chat.blocks.NoteRefs

/** The reader's path for a note link by id that names no note: its name is not known. */
const val UNKNOWN_NOTE = ""

/** The one action an Undo snackbar can take back. */
sealed interface PendingUndo {
    data object None : PendingUndo

    /** "Make a task" added task [id] for the note at [note]. */
    data class TaskAdded(val id: String, val note: String) : PendingUndo

    /** The note went to the Trash at [trashPath]. */
    data class NoteDeleted(val trashPath: String) : PendingUndo
}

/** A page pushed over the Notes home. Back pops one. */
sealed interface NotesRoute {
    data class Folder(val path: String) : NotesRoute
    data class Reader(val path: String) : NotesRoute
    data class Write(val path: String?) : NotesRoute
    data object Trash : NotesRoute
}

/** What the reader is showing for a note: still fetching, ready, gone (a 404), or HQ not answering. */
sealed interface ReaderPhase {
    data object Loading : ReaderPhase
    data class Ready(val note: NoteSheet, val edited: String) : ReaderPhase
    data object Gone : ReaderPhase
    data object Failed : ReaderPhase
}

data class TrashView(val loaded: Boolean = false, val failed: Boolean = false, val items: List<HqClient.TrashedNote> = emptyList(), val retentionDays: Int = 30, val open: Map<String, String> = emptyMap(), val busy: Set<String> = emptySet())

data class FolderView(val path: String, val folders: List<Pair<String, Int>> = emptyList(), val notes: List<String> = emptyList(), val loading: Boolean = true)

data class SearchHit(val path: String, val title: String, val notebook: String, val snippet: String)


enum class SaveStatus { Saved, Unsaved, Saving, Failed }

/** A save HQ refused: [theirs] is their text once the owner asked to see it. */
data class Conflict(val by: String?, val theirs: String? = null, val busy: Boolean = false)

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
    /** The hash the note had when the editor opened; null for a new note or an older HQ. */
    val base: String? = null,
    /** Set when HQ refused the save because the note changed elsewhere. */
    val conflict: Conflict? = null,
) {
    val isNew: Boolean get() = path == null
    val dirty: Boolean get() = text.text != original || (isNew && title.isNotBlank())

    /** The note as the leave dialog names it. */
    val leaveName: String get() = title.ifBlank { path?.substringAfterLast('/')?.removeSuffix(".md").orEmpty() }
}

data class NotesUi(
    val loaded: Boolean = false,
    val offline: Boolean = false,
    val recent: List<NoteItem> = emptyList(),
    val pinned: List<NoteItem> = emptyList(),
    val folders: List<String> = emptyList(),
    val stack: List<NotesRoute> = emptyList(),
    val folder: FolderView? = null,
    val reader: ReaderPhase = ReaderPhase.Loading,
    val write: WriteState? = null,
    val menu: Boolean = false,
    val export: ExportState? = null,
    val notice: Notice? = null,
    val trash: TrashView = TrashView(),
    /** What the one Undo snackbar on screen would take back. A new action replaces it, so the line and the Undo never disagree. */
    val pending: PendingUndo = PendingUndo.None,
    /** Notes the owner opened or kept on this phone: a refresh that was already in flight must not bring their New or Changed mark back. */
    val seenLocally: Set<String> = emptySet(),
    /** How many notes the Trash holds, for its chip; zero draws no number. */
    val trashCount: Int = 0,
    /** HQ filed or changed something the owner has not seen, for the dot on Vault. */
    val hasNew: Boolean = false,
    /** The dot was cleared by opening Vault; a note HQ files later brings it back. */
    val dotCleared: Boolean = false,
    /** The full folder list opened by the More chip. */
    val foldersOpen: Boolean = false,
    /** The New sheet. */
    val newOpen: Boolean = false,
    /** The first read is taking longer than [VaultRules.SLOW_MS]. */
    val slow: Boolean = false,
    /** The last read failed with HQ answering badly, as opposed to HQ being off ([offline]). */
    val error: Boolean = false,
) {

    val top: NotesRoute? get() = stack.lastOrNull()

    /** The quiet dot on Vault in the bar. Never a number. */
    val dot: Boolean get() = hasNew && !dotCleared

    /** The folders directly under Notebooks, Inbox first, for the chips on the home screen. */
    val topFolders: List<String> get() = folders.map { it.removePrefix("Notebooks/").substringBefore('/') }.filter { it.isNotBlank() }.distinct()
}

/** The task "Make a task" adds: the note's title, its path as the description (a task has no link field), and the path as the repeat key. */
internal fun noteTaskBody(path: String, title: String): org.json.JSONObject =
    org.json.JSONObject().put("title", title.ifBlank { NoteRefs.title(path) }).put("description", path).put("space_id", "personal").put("initiative", "Inbox").put("created_by", "operator").put("external_id", "note:$path")
