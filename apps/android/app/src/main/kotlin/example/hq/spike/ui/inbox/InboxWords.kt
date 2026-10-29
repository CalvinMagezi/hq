package example.hq.spike.ui.inbox

import example.hq.spike.data.HqClient
import example.hq.spike.data.HqClient.InboxItem
import example.hq.spike.ui.chat.blocks.Plain
import example.hq.spike.ui.helpers.AskKind
import example.hq.spike.ui.helpers.HelperWords
import java.time.Duration
import java.time.Instant
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.DateTimeParseException

enum class CardKind { HelperAsks, TaskReview, Approval, HelperStuck, Update }

enum class CardIcon { Sparkle, Info, Pause, Check, Task, Warning }

/** An inbox row in the owner's words. [sessionId] is set for a helper at a prompt, which is answered by typing keys into it first. */
data class InboxCard(
    val id: String,
    val kind: CardKind,
    val title: String,
    val body: String,
    val where: String,
    val createdAt: String,
    val icon: CardIcon,
    val sessionId: String?,
    val canAllow: Boolean,
    val allowLabel: String,
    val link: String?,
    val asked: String,
    val state: String,
    /** For a task waiting to be checked: its short id (such as FR-12), to open it in Tasks. */
    val taskRef: String? = null,
    /** For a helper at a prompt: the lines the notice quoted from its screen. An answer is sent only if the helper's screen still shows this dialog. */
    val excerpt: List<String> = emptyList(),
    /** For an approval: everything it says. The card shows [body], cut short; the details page shows this, and answers from there. */
    val fullBody: String = "",
    /** For a helper's question the server read: the dialog the answer goes to. Answered through the server, never by keys typed here. */
    val dialog: HqClient.AlertDialog? = null,
)

/** Turns what the server stores into the plain titles, bodies and buttons of the Inbox board. */
object InboxWords {
    private val HELPER_TITLE = Regex("^Harness session '(.*)' (is waiting for you|needs an answer|keeps showing a prompt)$")
    private val SESSION = Regex("Session (\\S+) on (\\S+) is (?:blocked on a prompt or approval|waiting at a prompt)")
    private const val TASK_REVIEW_SOURCE = "task_ready_for_review"
    private val TASK_PREFIX = Regex("^[A-Za-z][A-Za-z0-9]*(?:-[A-Za-z0-9]+)*-\\d+:\\s+")
    private const val MAX_BODY = 160
    private const val APPROVAL_BODY = 400

    fun card(item: InboxItem): InboxCard {
        val helper = HELPER_TITLE.matchEntire(item.title.trim())
        val session = SESSION.find(item.description)
        return when {
            helper != null -> helperCard(item, helper.groupValues[1], helper.groupValues[2], session?.groupValues?.get(1), session?.groupValues?.get(2))
            item.kind == "action_needed" && item.sourceTask == TASK_REVIEW_SOURCE -> {
                // Saying yes here only clears the notice, so it is not offered as "Allow": the owner is asked to look at the task.
                val body = Plain.strip(item.description.trim()).let { if (it.length > APPROVAL_BODY) it.take(APPROVAL_BODY).trimEnd() + "…" else it }
                InboxCard(item.id, CardKind.TaskReview, Plain.strip(item.title.trim()).replace(TASK_PREFIX, ""), body, "Task ready to check", item.createdAt, CardIcon.Task, null, false, "Check it", item.link, askedBy(item, null), item.state, taskRef = TASK_PREFIX.find(item.title.trim())?.value?.trim()?.removeSuffix(":"))
            }
            item.actions.any { it == "approve" || it == "reject" } -> {
                val full = Plain.strip(item.description.trim())
                val body = if (full.length > APPROVAL_BODY) full.take(APPROVAL_BODY).trimEnd() + "…" else full
                InboxCard(item.id, CardKind.Approval, Plain.strip(item.title.trim()), body, whereOf(item, null), item.createdAt, CardIcon.Info, null, full.length <= APPROVAL_BODY, "Allow", item.link, askedBy(item, null), item.state, fullBody = full)
            }
            else -> InboxCard(item.id, CardKind.Update, Plain.strip(item.title.trim()), Plain.strip(item.description.trim()), whereOf(item, null), item.createdAt, updateIcon(item), null, false, "", item.link, askedBy(item, null), item.state)
        }
    }

    private fun helperCard(item: InboxItem, label: String, what: String, sessionId: String?, host: String?): InboxCard {
        val who = label.trim().split(' ', '(').firstOrNull { it.isNotBlank() }?.replaceFirstChar { it.uppercase() } ?: "A helper"
        val line = lastLine(item.description)
        val dialog = item.dialog?.takeIf { !it.answered }
        if (what == "is waiting for you" && dialog != null) return dialogCard(item, who, dialog, host)
        return when (what) {
            "is waiting for you" -> {
                val excerpt = HelperWords.excerptLines(item.description)
                val ask = HelperWords.blocked(excerpt)
                // Answered from the card only when the card shows everything the owner needs to decide, and Enter would take a plain
                // yes once (the tap checks the same again on the live screen); otherwise the helper's own page does.
                val inline = sessionId != null && ask != null && HelperWords.canAnswerInline(ask) && HelperWords.enterMeansOnce(excerpt)
                val body = when {
                    ask == null -> if (line.isNotEmpty()) "It stopped to ask: $line" else "It stopped to ask before going on."
                    ask.kind == AskKind.Other -> "It stopped to ask: ${ask.question}"
                    else -> HelperWords.askText(ask)
                }
                InboxCard(
                    item.id, CardKind.HelperAsks, "$who needs your OK", body, whereOf(item, host), item.createdAt, CardIcon.Sparkle, sessionId, inline,
                    if (inline) "Allow once" else "Open helper", item.link, askedBy(item, host), item.state, excerpt = excerpt,
                )
            }
            "keeps showing a prompt" -> InboxCard(
                item.id, CardKind.HelperStuck, "A helper is stuck", "$who keeps hitting the same question, so HQ stopped answering it.",
                whereOf(item, host), item.createdAt, CardIcon.Pause, sessionId, false, "Take a look", item.link, askedBy(item, host), item.state,
            )
            else -> InboxCard(
                item.id, CardKind.HelperStuck, "$who needs an answer", "It is waiting in its chat.",
                whereOf(item, host), item.createdAt, CardIcon.Pause, sessionId, false, "Take a look", item.link, askedBy(item, host), item.state,
            )
        }
    }

    /**
     * A helper's question the server read: the card says what it asks and the command or path it is about, and offers Allow
     * once only when the server named a plain yes-once answer. The answer goes to the server, which sends it to that dialog
     * and no other, so nothing here reads the screen.
     */
    private fun dialogCard(item: InboxItem, who: String, d: HqClient.AlertDialog, host: String?): InboxCard {
        val body = listOfNotNull(d.question, d.command).joinToString("\n").ifBlank { "It stopped to ask before going on." }
        val canAllow = d.approveLabel != null
        return InboxCard(
            item.id, CardKind.HelperAsks, "$who needs your OK", body, whereOf(item, host), item.createdAt, CardIcon.Sparkle, d.sessionId, canAllow,
            if (canAllow) "Allow once" else "Open helper", item.link, askedBy(item, host), item.state, dialog = d,
        )
    }

    /** What the helper is asking, from the screen excerpt the supervisor includes. */
    internal fun lastLine(description: String): String {
        val lines = description.lines().map { it.trim() }.filter { it.isNotEmpty() }
        val readIt = lines.indexOfFirst { it.startsWith("Read it with") }
        val before = if (readIt >= 0) lines.take(readIt) else lines
        val candidates = before.drop(1).filter { !it.startsWith("<") && !it.startsWith("```") && it.any(Char::isLetter) }
        // The question, not the choices under it: the last line that asks something, else the last line.
        val line = (candidates.lastOrNull { it.endsWith("?") } ?: candidates.lastOrNull()).orEmpty()
        return if (line.length > MAX_BODY) line.take(MAX_BODY).trimEnd() + "…" else line
    }

    private fun whereOf(item: InboxItem, host: String?): String = host ?: when (val s = item.sourceTask) {
        null, "" -> "HQ"
        "session-supervisor", "session_chat" -> "A helper"
        else -> s.replace('-', ' ').replace('_', ' ').replaceFirstChar { it.uppercase() }
    }

    private fun askedBy(item: InboxItem, host: String?): String = whereOf(item, host)

    private fun updateIcon(item: InboxItem): CardIcon {
        val t = item.title.lowercase()
        return when {
            listOf("finished", "done", "saved", "complete").any { it in t } -> CardIcon.Check
            listOf("ready to check", "task").any { it in t } -> CardIcon.Task
            listOf("low", "failed", "error", "stuck", "offline", "full").any { it in t } -> CardIcon.Warning
            else -> CardIcon.Info
        }
    }

    private fun parse(value: String): Instant? = try { OffsetDateTime.parse(value).toInstant() } catch (e: DateTimeParseException) { null }

    /** "now", "2 min ago", "1 hour ago", "3 days ago": the age in a card's meta line. */
    fun ago(createdAt: String, now: Instant = Instant.now()): String {
        val then = parse(createdAt) ?: return ""
        val minutes = Duration.between(then, now).toMinutes().coerceAtLeast(0)
        return when {
            minutes < 1 -> "now"
            minutes < 60 -> "$minutes min ago"
            minutes < 120 -> "1 hour ago"
            minutes < 24 * 60 -> "${minutes / 60} hours ago"
            minutes < 48 * 60 -> "yesterday"
            else -> "${minutes / (24 * 60)} days ago"
        }
    }

    /** 14:02, in the phone's own time zone, for the end of an update row. */
    fun clock(createdAt: String, zone: ZoneId = ZoneId.systemDefault()): String =
        parse(createdAt)?.atZone(zone)?.toLocalTime()?.format(DateTimeFormatter.ofPattern("HH:mm")).orEmpty()

    /** "Today", "Yesterday", or "Mon 19 Oct": the heading over a day of rows. */
    fun dayLabel(createdAt: String, now: Instant = Instant.now(), zone: ZoneId = ZoneId.systemDefault()): String {
        val day = parse(createdAt)?.atZone(zone)?.toLocalDate() ?: return "Earlier"
        val today = now.atZone(zone).toLocalDate()
        return when (day) {
            today -> "Today"
            today.minusDays(1) -> "Yesterday"
            else -> day.format(DateTimeFormatter.ofPattern("EEE d MMM", java.util.Locale.getDefault()))
        }
    }

    fun sameDay(createdAt: String, now: Instant = Instant.now(), zone: ZoneId = ZoneId.systemDefault()): Boolean =
        parse(createdAt)?.atZone(zone)?.toLocalDate() == now.atZone(zone).toLocalDate()
}
