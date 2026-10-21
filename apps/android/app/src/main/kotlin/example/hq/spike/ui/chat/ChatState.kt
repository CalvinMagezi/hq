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
    /** Vault paths attached to the next message, shown as chips above the field. */
    val contexts: List<String> = emptyList(),
) {
    val running: Boolean get() = live != null
    val canSend: Boolean get() = draft.isNotBlank() && connected && !running

    companion object {
        const val NEW_CHAT = "New chat"
    }
}

sealed interface Page {
    data class Chart(val spec: ChartSpec) : Page
    data class Table(val table: Block.Table, val caption: String) : Page
}

/** A note open in the sheet. [savedAge] is "just now", "3m ago" and so on when a reply in this chat wrote it, else null. */
data class NoteSheet(val path: String, val title: String, val trail: String, val body: String, val truncated: Boolean, val savedAge: String?)

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
        val messages = when {
            mine && saved != null -> state.messages.map { if (it.id == e.clientId) saved else it }
            saved != null && state.messages.none { it.id == saved.id } -> state.messages + saved
            else -> state.messages
        }
        return state.copy(threadId = e.threadId, messages = messages, live = LiveReply(), failed = false)
    }

    private fun reply(state: ChatUiState, event: ChatEvent): ChatUiState {
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
            messages = if (empty) state.messages else state.messages + message,
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

    private fun parse(value: String): Instant? = try { Instant.parse(value) } catch (e: DateTimeParseException) { null }

    private const val WEEK_DAYS = 7L
    private const val MINUTES_PER_HOUR = 60L
    private const val HOURS_PER_DAY = 24L
}

/** The server strips a leading `<hq-context>` block from the saved copy; the model still reads it. */
internal fun withContext(text: String, notes: List<String>): String =
    if (notes.isEmpty()) text else "<hq-context>The user attached these notes. Read each one before you answer: ${notes.joinToString(", ")}</hq-context>\n\n$text"
