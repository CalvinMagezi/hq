package example.hq.spike.ui.chat

import example.hq.spike.data.ChatEvent
import example.hq.spike.data.ToolStep
import example.hq.spike.ui.chat.blocks.Attachment
import example.hq.spike.ui.chat.blocks.Attachments
import example.hq.spike.ui.chat.blocks.Block
import example.hq.spike.ui.chat.blocks.ChartSpec
import java.time.Duration
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeParseException

enum class Speaker { User, Reply }

data class UiMessage(
    val id: String,
    val speaker: Speaker,
    val text: String,
    val tools: List<ToolStep> = emptyList(),
    /** True for a question shown at once, before the server has saved it. */
    val sending: Boolean = false,
    /** When the server saved it; blank for a reply that just finished in this session. */
    val createdAt: String = "",
    val files: List<Attachment> = emptyList(),
) {
    companion object {
        /** A saved message: the attachment marker the server appends is split off into [files]. */
        fun saved(id: String, speaker: Speaker, content: String, tools: List<ToolStep> = emptyList(), createdAt: String = ""): UiMessage {
            // Only the user's own messages carry the marker; a model reply that contains one is just text.
            if (speaker != Speaker.User) return UiMessage(id, speaker, content, tools, createdAt = createdAt)
            val (text, files) = Attachments.split(content)
            return UiMessage(id, speaker, text, tools, createdAt = createdAt, files = files)
        }
    }
}

data class LiveStep(val step: ToolStep, val done: Boolean) {
    val id: String get() = step.id
    val label: String get() = ToolWords.running(step)
}

/** The reply that is being written right now. */
data class LiveReply(val text: String = "", val steps: List<LiveStep> = emptyList()) {
    val headline: String get() = steps.lastOrNull { !it.done }?.let { "${it.label}..." } ?: "Working..."
}

enum class Group(val label: String) { Today("Today"), Yesterday("Yesterday"), Week("Previous 7 days"), Older("Older") }

data class ChatRow(val id: String, val title: String, val subtitle: String, val group: Group, val running: Boolean, val unread: Boolean)

data class ChatUiState(
    val threadId: String? = null,
    val title: String = NEW_CHAT,
    val messages: List<UiMessage> = emptyList(),
    val live: LiveReply? = null,
    /** The last reply did not finish; the card offers Retry. */
    val failed: Boolean = false,
    val connected: Boolean = true,
    /** When the socket dropped (epoch milliseconds), so the status line can say how old what is shown is. */
    val offlineSince: Long? = null,
    val draft: String = "",
    val notice: Notice? = null,
    val chatsOpen: Boolean = false,
    val chats: List<ChatRow> = emptyList(),
    val chatSearch: String = "",
    /** A chart or a table on its own page, over the whole shell. */
    val page: Page? = null,
    val note: NoteSheet? = null,
    /** The note being fetched after a tap, so its chip can say "Opening…". */
    val openingNote: String? = null,
    val missingNotes: Set<String> = emptySet(),
    /** Notes and tasks attached to the next message, shown as chips above the field. */
    val contexts: List<ContextRef> = emptyList(),
    val files: List<PendingFile> = emptyList(),
    val addOpen: Boolean = false,
    val picker: Picker? = null,
    /** The message whose long-press sheet is open. */
    val pressed: String? = null,
    /** A saved question being edited; the next send replaces it and what came after. */
    val editing: String? = null,
    /** Older messages are still on the server beyond what is loaded. */
    val hasOlder: Boolean = false,
    val noteMenu: Boolean = false,
    val export: ExportState? = null,
) {
    val running: Boolean get() = live != null
    val canSend: Boolean get() = (draft.isNotBlank() || files.isNotEmpty()) && connected && !running && files.all { it.status == FileStatus.Ready }

    companion object {
        const val NEW_CHAT = "New chat"
    }
}

/** A note or task attached to the next message. [ref] is the vault path of a note or the display id of a task. */
data class ContextRef(val kind: Kind, val ref: String, val label: String) {
    enum class Kind { Note, Task }
}

enum class FileStatus { Uploading, Ready, Failed }

/** A file picked for the next message, from pick to upload result. [uri] stays so a failed upload can be tried again. */
data class PendingFile(
    val id: String,
    val uri: String,
    val name: String,
    val size: Long,
    val mime: String,
    val status: FileStatus,
    val error: String? = null,
    val uploaded: Attachment? = null,
) {
    /** A failed upload can be tried again; a file that is too big or empty cannot. */
    val retryable: Boolean get() = status == FileStatus.Failed && uploaded == null && error == UPLOAD_FAILED

    companion object {
        const val UPLOAD_FAILED = "upload failed"
    }
}

/** The note or task picker: what is typed and what matched. */
data class Picker(val kind: ContextRef.Kind, val query: String = "", val results: List<ContextRef> = emptyList(), val loading: Boolean = true)

sealed interface Page {
    data class SelectText(val text: String) : Page
    data class Chart(val spec: ChartSpec) : Page
    data class Table(val table: Block.Table, val caption: String) : Page
}

/** A note open in the sheet. [savedAge] is "just now", "3m ago" and so on when a reply in this chat wrote it, else null. */
data class NoteSheet(
    val path: String,
    val title: String,
    val trail: String,
    val body: String,
    val truncated: Boolean,
    val savedAge: String?,
    val pinned: Boolean = false,
)

enum class ExportGroup(val label: String) { Document("Document"), Picture("Picture"), Tables("Tables in this note") }

/** The formats the web app offers, in the board's order, each with the plain words under it. [word] is how a message names the file ("Making your PDF"). */
enum class ExportFormat(val key: String, val label: String, val blurb: String, val ext: String, val group: ExportGroup, val word: String) {
    Pdf("pdf", "PDF document", "Best for printing and sending", ".pdf", ExportGroup.Document, "PDF"),
    Docx("docx", "Word document", "Opens in Word", ".docx", ExportGroup.Document, "Word document"),
    Html("html", "Web page", "Opens in a browser", ".html", ExportGroup.Document, "web page"),
    Markdown("md", "Markdown", "Plain text with the marks", ".md", ExportGroup.Document, "Markdown file"),
    Png("png", "Whole note as one image", "One tall picture", ".png", ExportGroup.Picture, "picture"),
    Svg("svg", "Sharp at any size", "A vector image", ".svg", ExportGroup.Picture, "image"),
    Xlsx("xlsx", "Excel workbook", "Opens in Excel", ".xlsx", ExportGroup.Tables, "Excel workbook"),
    Csv("csv", "CSV", "Opens in any spreadsheet", ".csv", ExportGroup.Tables, "CSV file"),
    Json("json", "JSON", "For programs", ".json", ExportGroup.Tables, "JSON file"),
}

data class ExportNote(val path: String, val title: String, val tables: Int)

/** Where an export stands: picking a format, being made on hq, ready to send or keep, or failed. */
sealed interface ExportState {
    val note: ExportNote

    data class Choosing(override val note: ExportNote) : ExportState
    data class Making(override val note: ExportNote, val format: ExportFormat) : ExportState
    data class Ready(override val note: ExportNote, val format: ExportFormat, val name: String, val mime: String, val file: String, val size: Long) : ExportState
    data class Failed(override val note: ExportNote, val format: ExportFormat, val reason: String) : ExportState
}

/** A line at the bottom of the screen, with an optional action word (Undo). */
data class Notice(val text: String, val action: String? = null)

/** Applies one server event to the open chat. Pure, so the rules can be tested without a phone. */
object ChatReducer {
    fun apply(state: ChatUiState, event: ChatEvent, pendingClientIds: Set<String>): ChatUiState = when (event) {
        is ChatEvent.TurnStart -> turnStart(state, event, pendingClientIds)
        is ChatEvent.Titled -> if (event.threadId == state.threadId) state.copy(title = event.title) else state
        else -> if (event.threadOf() != null && event.threadOf() == state.threadId) reply(state, event) else state
    }

    private fun turnStart(state: ChatUiState, e: ChatEvent.TurnStart, pending: Set<String>): ChatUiState {
        val mine = e.clientId != null && e.clientId in pending
        val here = e.threadId != null && (e.threadId == state.threadId || (state.threadId == null && mine))
        if (!here) return state
        val saved = e.userMessage?.let { UiMessage.saved(it.id, Speaker.User, it.content) }
        // An edit made in another tab or on another device replaced a question and what followed it.
        val base = if (!mine && e.replaceFrom != null) state.messages.takeWhile { it.id != e.replaceFrom } else state.messages
        val messages = when {
            mine && saved != null -> base.map { if (it.id == e.clientId) saved else it }.distinctBy { it.id }
            saved != null && base.none { it.id == saved.id } -> base + saved
            else -> base
        }
        return state.copy(threadId = e.threadId, messages = messages, live = LiveReply(), failed = false)
    }

    private fun reply(state: ChatUiState, event: ChatEvent): ChatUiState {
        // A turn_end with no turn running here is a message the server posted (a helper's update): the caller reloads the chat.
        if (event is ChatEvent.TurnEnd && state.live == null) return state
        val live = state.live ?: LiveReply()
        return when (event) {
            is ChatEvent.Text -> state.copy(live = live.copy(text = live.text + event.text))
            is ChatEvent.ToolStarted -> state.copy(
                live = live.copy(steps = live.steps + LiveStep(event.step, false)),
            )
            is ChatEvent.ToolFinished -> state.copy(
                live = live.copy(steps = live.steps.map { if (it.id == event.toolCallId) it.copy(done = true) else it }),
            )
            is ChatEvent.TurnEnd -> finish(state, live, event)
            is ChatEvent.Failed -> state.copy(failed = true)
            else -> state
        }
    }

    private fun finish(state: ChatUiState, live: LiveReply, e: ChatEvent.TurnEnd): ChatUiState {
        val steps = live.steps.map { it.step }
        val empty = live.text.isBlank() && steps.isEmpty()
        val message = UiMessage(e.messageId ?: "reply-${state.messages.size}", Speaker.Reply, live.text, steps)
        return state.copy(
            live = null,
            messages = if (empty || state.messages.any { it.id == message.id }) state.messages else state.messages + message,
            failed = state.failed || (empty && !e.stopped),
        )
    }

    private fun ChatEvent.threadOf(): String? = when (this) {
        is ChatEvent.Text -> threadId
        is ChatEvent.Reasoning -> threadId
        is ChatEvent.ToolStarted -> threadId
        is ChatEvent.ToolProgress -> threadId
        is ChatEvent.ToolFinished -> threadId
        is ChatEvent.TurnEnd -> threadId
        is ChatEvent.Failed -> threadId
        else -> null
    }
}

/** The plain words the design uses for what a reply did, instead of tool names. */
object ToolWords {
    fun running(step: ToolStep): String = when (step.name) {
        "task_list" -> "Checked your tasks"
        "task_create" -> "Added a task"
        "vault_read" -> noteTitle(step.args)?.let { "Read \"$it\"" } ?: "Read a note"
        "vault_search" -> "Searched your notes"
        "vault_write_note" -> "Saved a note"
        "bash", "run_command" -> "Worked on your computer"
        else -> "Did a step"
    }

    private fun noteTitle(args: String?): String? {
        val path = Regex("\"path\"\\s*:\\s*\"([^\"]+)\"").find(args ?: "")?.groupValues?.get(1) ?: return null
        return path.substringAfterLast('/').removeSuffix(".md")
    }
}

/** Time words for the chats sheet. */
object ChatTime {
    fun group(updatedAt: String, now: Instant = Instant.now(), zone: ZoneId = ZoneId.systemDefault()): Group {
        val day = parse(updatedAt)?.atZone(zone)?.toLocalDate() ?: return Group.Older
        val today = now.atZone(zone).toLocalDate()
        return when {
            day == today -> Group.Today
            day == today.minusDays(1) -> Group.Yesterday
            day.isAfter(today.minusDays(WEEK_DAYS)) -> Group.Week
            else -> Group.Older
        }
    }

    fun age(updatedAt: String, now: Instant = Instant.now()): String {
        val then = parse(updatedAt) ?: return ""
        val minutes = Duration.between(then, now).toMinutes()
        return when {
            minutes < 1 -> "now"
            minutes < MINUTES_PER_HOUR -> "${minutes}m ago"
            else -> "${minutes / MINUTES_PER_HOUR}h ago"
        }
    }

    /** "just now", "3m ago", "2h ago", "yesterday", "4d ago"; blank when the time is not known. */
    fun since(at: String, now: Instant = Instant.now(), zone: ZoneId = ZoneId.systemDefault()): String {
        val then = parse(at) ?: return ""
        val minutes = Duration.between(then, now).toMinutes()
        return when {
            minutes < 1 -> "just now"
            minutes < MINUTES_PER_HOUR -> "${minutes}m ago"
            minutes < MINUTES_PER_HOUR * HOURS_PER_DAY -> "${minutes / MINUTES_PER_HOUR}h ago"
            then.atZone(zone).toLocalDate() == now.atZone(zone).toLocalDate().minusDays(1) -> "yesterday"
            else -> "${minutes / (MINUTES_PER_HOUR * HOURS_PER_DAY)}d ago"
        }
    }

    /** How long ago, for the status line: "<1m", "3m", "2h", "5d". */
    fun short(sinceMillis: Long, nowMillis: Long): String {
        val minutes = Duration.ofMillis((nowMillis - sinceMillis).coerceAtLeast(0)).toMinutes()
        return when {
            minutes < 1 -> "<1m"
            minutes < MINUTES_PER_HOUR -> "${minutes}m"
            minutes < MINUTES_PER_HOUR * HOURS_PER_DAY -> "${minutes / MINUTES_PER_HOUR}h"
            else -> "${minutes / (MINUTES_PER_HOUR * HOURS_PER_DAY)}d"
        }
    }

    /** The time of day a message was saved, as 14:28; blank when the time is not known. */
    fun clock(at: String, zone: ZoneId = ZoneId.systemDefault()): String =
        parse(at)?.atZone(zone)?.toLocalTime()?.format(java.time.format.DateTimeFormatter.ofPattern("HH:mm")).orEmpty()

    /** hq writes `+00:00`, not `Z`; Instant.parse takes only `Z` before Android 14, so the offset form is read instead. */
    private fun parse(value: String): Instant? = try { java.time.OffsetDateTime.parse(value).toInstant() } catch (e: DateTimeParseException) { null }

    private const val WEEK_DAYS = 7L
    private const val MINUTES_PER_HOUR = 60L
    private const val HOURS_PER_DAY = 24L
}

/** The server strips a leading `<hq-context>` block from the saved copy; the model still reads it. */
internal fun withContext(text: String, items: List<ContextRef>): String {
    if (items.isEmpty()) return text
    val list = items.joinToString("; ") { if (it.kind == ContextRef.Kind.Note) "note ${it.ref}" else "task ${it.ref} (${it.label})" }
    return "<hq-context>The user attached these. Read each one before you answer: $list</hq-context>\n\n$text"
}

/** The two commands the web chat has: both archive this chat and open a fresh one, `/topic` with a name. */
object SlashCommands {
    data class Command(val name: String, val args: String?)

    val All = listOf(
        Triple("/clear", "", "Archive this chat and start a new one"),
        Triple("/topic", "<name>", "Archive this chat and start a new one with that name"),
    )

    fun parse(input: String): Command? {
        val t = input.trim()
        if (!t.startsWith("/")) return null
        val space = t.indexOf(' ')
        val name = (if (space < 0) t else t.substring(0, space)).lowercase()
        val args = if (space < 0) null else t.substring(space + 1).trim().ifEmpty { null }
        return if (All.any { it.first == name }) Command(name, args) else null
    }

    /** The commands to offer while the field starts with a slash and no space has been typed yet. */
    fun matching(draft: String): List<Triple<String, String, String>> =
        if (!draft.startsWith("/") || draft.contains(' ') || draft.contains('\n')) emptyList() else All.filter { it.first.startsWith(draft.lowercase()) }
}
