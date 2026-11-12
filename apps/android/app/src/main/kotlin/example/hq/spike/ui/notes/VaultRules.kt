package example.hq.spike.ui.notes

import example.hq.spike.data.HqClient.NoteItem
import example.hq.spike.data.RESTORED_VAULT
import example.hq.spike.data.WRITTEN_BY_HQ
import example.hq.spike.ui.chat.blocks.NoteRefs

/** What a row says about who wrote the note and whether the owner has looked: the only review words are New and Changed. */
enum class ReviewMark { None, New, Changed }

/** The rules of the Vault home and reader that need no screen, so a test can pin them. */
object VaultRules {
    /** Pinned cards on the home screen; more are one tap away in the search. */
    const val MAX_PINNED = 6

    /** Folder chips before the "More" chip. */
    const val MAX_FOLDER_CHIPS = 6

    /** How long the first read of the notes may take before the screen says it is still working. */
    const val SLOW_MS = 2_000L

    /** From this font scale up the pages stack and wrap instead of squeezing (the board's 200% frame). */
    const val LARGE_TEXT_SCALE = 1.5f

    /** HQ gave no answer at all (status 0) is "not running"; any answer the phone could not use is an error with Try again. */
    fun isOffline(status: Int): Boolean = status == 0

    fun pinnedShown(pinned: List<NoteItem>): List<NoteItem> = pinned.take(MAX_PINNED)

    /** The folder chips: [shown] in the row, [more] behind the More chip, and the Trash chip with its count (zero draws no number). */
    data class Chips(val shown: List<String>, val more: List<String>, val trash: Int) {
        val hasMore: Boolean get() = more.isNotEmpty()
    }

    fun chips(folders: List<String>, trashCount: Int): Chips = Chips(folders.take(MAX_FOLDER_CHIPS), folders.drop(MAX_FOLDER_CHIPS), trashCount.coerceAtLeast(0))

    fun writtenByHq(item: NoteItem): Boolean = item.writtenBy == WRITTEN_BY_HQ

    fun markOf(review: String?): ReviewMark = when (review) {
        "new" -> ReviewMark.New
        "changed" -> ReviewMark.Changed
        else -> ReviewMark.None
    }

    /**
     * What a note is called: the `title` in its front matter, else its first top heading, else its file name. The task
     * "Make a task" adds is named with this, never with the file name.
     */
    fun titleOf(content: String, path: String): String {
        val lines = content.lineSequence().toList()
        val fenced = lines.firstOrNull()?.trim() == "---"
        var body = lines
        if (fenced) {
            val end = lines.drop(1).indexOfFirst { it.trim() == "---" }
            if (end >= 0) {
                lines.subList(1, end + 1).firstNotNullOfOrNull { TITLE_LINE.matchEntire(it.trim())?.groupValues?.get(1)?.trim()?.trim('"', '\'')?.takeIf(String::isNotBlank) }?.let { return it }
                body = lines.drop(end + 2)
            }
        }
        body.firstNotNullOfOrNull { HEADING.matchEntire(it.trim())?.groupValues?.get(1)?.trim()?.takeIf(String::isNotBlank) }?.let { return it }
        return NoteRefs.title(path)
    }

    /** Where a restored note went, as a folder name for the line, or null when it went to the top of the Vault. */
    fun restoredPlace(restoredTo: String, path: String): String? {
        if (restoredTo == RESTORED_VAULT) return null
        val folder = path.removePrefix("Notebooks/").substringBeforeLast('/', "")
        return folder.substringAfterLast('/').ifBlank { null }
    }

    /** Whether the dot is cleared after a read: it is while Vault is open, and it stays so until a note the phone had not heard of arrives. */
    fun dotCleared(visible: Boolean, fresh: Boolean, cleared: Boolean): Boolean = visible || (cleared && !fresh)

    /** Which notes in [recent] are marked and were not before: the dot returns only for these. */
    fun markedPaths(recent: List<NoteItem>): Set<String> = recent.filter { it.review != null }.map { it.path }.toSet()

    fun hasFresh(known: Set<String>, now: Set<String>): Boolean = now.any { it !in known }

    private val TITLE_LINE = Regex("title\\s*:\\s*(.+)", RegexOption.IGNORE_CASE)
    private val HEADING = Regex("#\\s+(.+)")
}

/**
 * The state changes the Vault view model makes, kept apart from its coroutines so each can be tested. They only read
 * their arguments, so a retried state update gives the same result.
 */
object VaultReducer {
    /** A new action takes the snackbar: whatever was pending is replaced, not kept beside it. */
    fun pend(ui: NotesUi, undo: PendingUndo): NotesUi = ui.copy(pending = undo)

    /** Undo was tapped: what it takes back (or None), and the state with the snackbar cleared. */
    fun undone(ui: NotesUi): Pair<PendingUndo, NotesUi> = ui.pending to ui.copy(pending = PendingUndo.None)

    /** The snackbar timed out: nothing to take back any more. */
    fun expired(ui: NotesUi): NotesUi = ui.copy(pending = PendingUndo.None)

    /** The owner opened (or kept) [path]: its mark goes at once, and stays gone if an older answer arrives later. */
    fun seen(ui: NotesUi, path: String): NotesUi = ui.copy(seenLocally = ui.seenLocally + path, recent = unmarked(ui.recent, setOf(path)))

    /**
     * Signals from HQ arrived. Notes the owner has seen on this phone are shown unmarked even if the answer was asked
     * for before they were opened. A seen note HQ now reports as unmarked is forgotten, so a later change shows again.
     */
    fun signals(ui: NotesUi, recent: List<NoteItem>, hasNew: Boolean, dotCleared: Boolean): NotesUi {
        val confirmed = recent.filter { it.review == null }.map { it.path }.toSet()
        val keep = ui.seenLocally - confirmed
        val marked = recent.filter { it.review != null }
        // The flag may be there only because of notes this phone has since opened.
        val stale = marked.isNotEmpty() && marked.all { it.path in keep }
        return ui.copy(recent = unmarked(recent, keep), hasNew = hasNew && !stale, dotCleared = dotCleared, seenLocally = keep)
    }

    private fun unmarked(list: List<NoteItem>, paths: Set<String>) = list.map { if (it.path in paths && it.review != null) it.copy(review = null) else it }
}

/** The one photo being taken: its file name is kept across a rotation, and ends either handed to the chat or thrown away. */
class PhotoShot(var pending: String? = null) {
    class Ended(val handOver: String?, val discard: String?)

    fun begin(name: String) { pending = name }

    /** The camera came back. A taken photo goes on; a cancelled or failed one is to be deleted. A result with nothing pending (a second callback) does nothing. */
    fun finish(taken: Boolean): Ended {
        val name = pending
        pending = null
        return if (taken && name != null) Ended(name, null) else Ended(null, name)
    }
}

/** Photos wait in the cache only long enough for the chat to upload them. */
object CaptureFiles {
    const val KEEP_MS = 60 * 60 * 1000L

    /** The names to delete: files older than [KEEP_MS], never the one being taken now. */
    fun stale(files: List<Pair<String, Long>>, now: Long, current: String? = null): List<String> =
        files.filter { (name, modified) -> name != current && now - modified > KEEP_MS }.map { it.first }
}
