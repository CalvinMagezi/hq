package example.hq.spike.ui.tasks

import example.hq.spike.data.HqClient.TaskItem
import java.time.DayOfWeek
import java.time.Duration
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import java.time.format.DateTimeParseException
import java.time.temporal.TemporalAdjusters
import java.util.Locale

enum class TaskTab { Today, Upcoming, All }

/**
 * A heading and the tasks under it. The badge counts them; [collapsible] groups (Later, No date) show only their count until tapped;
 * a group with a [preview] shows that many and a row to show the rest.
 */
data class TaskGroup(val key: String, val title: String, val tasks: List<TaskItem>, val badge: Boolean = true, val collapsible: Boolean = false, val preview: Int? = null)

/** One piece of a row's second line: some words, bold or not, plain or highlighted. */
data class Run(val text: String, val bold: Boolean = false, val amber: Boolean = false)

object TasksLogic {
    private const val SOON_DAYS = 7L
    const val WAITING_PREVIEW = 5
    private val PRIORITY_ORDER = listOf("urgent", "high", "normal", "low")

    fun parse(date: String?): LocalDate? = try { date?.let(LocalDate::parse) } catch (e: DateTimeParseException) { null }

    private fun open(t: TaskItem) = t.status != "complete" && t.parentId == null

    /** Priority first, then the oldest. */
    val byUrgency: Comparator<TaskItem> = compareBy<TaskItem>({ PRIORITY_ORDER.indexOf(it.priority).let { i -> if (i < 0) PRIORITY_ORDER.size else i } }, { it.updatedAt })

    /** The date a task belongs to in the lists: when it starts, else when it is due. */
    private fun when_(t: TaskItem): LocalDate? = parse(t.startDate) ?: parse(t.dueDate)

    fun today(tasks: List<TaskItem>, today: LocalDate): List<TaskGroup> {
        val live = tasks.filter(::open)
        val waiting = live.filter { it.status == "blocked" || it.status == "ready_for_review" }.sortedWith(byUrgency)
        val going = live.filter { it.status == "in_progress" }.sortedWith(byUrgency)
        val due = live.filter { it.status == "to_do" }.filter { t -> listOfNotNull(parse(t.startDate), parse(t.dueDate)).any { !it.isAfter(today) } }.sortedWith(byUrgency)
        return listOf(
            TaskGroup("waiting", "Waiting on you", waiting, preview = WAITING_PREVIEW), TaskGroup("going", "In progress", going), TaskGroup("due", "Due or starting today", due),
        ).filter { it.tasks.isNotEmpty() }
    }

    fun upcoming(tasks: List<TaskItem>, today: LocalDate): List<TaskGroup> {
        val live = tasks.filter(::open).filter { it.status == "to_do" || it.status == "in_progress" }
        val dated = live.mapNotNull { t -> when_(t)?.takeIf { it.isAfter(today) }?.let { it to t } }
        val soon = dated.filter { (d, _) -> !d.isAfter(today.plusDays(SOON_DAYS)) }.groupBy({ it.first }, { it.second }).toSortedMap()
        val groups = soon.map { (d, ts) -> TaskGroup("d-$d", dayLabel(d, today), ts.sortedWith(byUrgency), badge = false) }.toMutableList()
        val later = dated.filter { (d, _) -> d.isAfter(today.plusDays(SOON_DAYS)) }.map { it.second }.sortedWith(byUrgency)
        if (later.isNotEmpty()) groups += TaskGroup("later", "Later · ${plural(later.size)}", later, badge = false, collapsible = true)
        val undated = live.filter { when_(it) == null && it.status == "to_do" }.sortedWith(byUrgency)
        if (undated.isNotEmpty()) groups += TaskGroup("nodate", "No date · ${plural(undated.size)}", undated, badge = false, collapsible = true)
        return groups
    }

    fun all(tasks: List<TaskItem>): List<TaskGroup> {
        val live = tasks.filter(::open)
        return listOf(
            TaskGroup("going", "In progress", live.filter { it.status == "in_progress" }.sortedWith(byUrgency)),
            TaskGroup("blocked", "Stuck", live.filter { it.status == "blocked" }.sortedWith(byUrgency)),
            TaskGroup("review", "Ready to check", live.filter { it.status == "ready_for_review" }.sortedWith(byUrgency)),
            TaskGroup("todo", "To do", live.filter { it.status == "to_do" }.sortedWith(byUrgency)),
        ).filter { it.tasks.isNotEmpty() }
    }

    private fun plural(n: Int) = if (n == 1) "1 task" else "$n tasks"

    private fun dayLabel(d: LocalDate, today: LocalDate): String {
        val long = d.format(DateTimeFormatter.ofPattern("EEE d MMM", Locale.getDefault()))
        return if (d == today.plusDays(1)) "Tomorrow · $long" else long
    }

    /** "due today", "due tomorrow", "due Thursday" within a week, else "due Oct 24". */
    fun dueWords(due: String?, today: LocalDate): String? {
        val d = parse(due) ?: return null
        return when {
            d == today -> "due today"
            d == today.plusDays(1) -> "due tomorrow"
            d.isBefore(today) -> "overdue since " + d.format(DateTimeFormatter.ofPattern("MMM d", Locale.getDefault()))
            !d.isAfter(today.plusDays(SOON_DAYS - 1)) -> "due " + d.format(DateTimeFormatter.ofPattern("EEEE", Locale.getDefault()))
            else -> "due " + d.format(DateTimeFormatter.ofPattern("MMM d", Locale.getDefault()))
        }
    }

    /** Why a to-do is on the Today list when it is not due today: it starts today, or it started earlier and is still open. */
    fun startedWords(t: TaskItem, today: LocalDate): String? {
        val start = parse(t.startDate) ?: return null
        if (t.status != "to_do" || parse(t.dueDate) == today) return null
        return when {
            start == today -> "starts today"
            start.isBefore(today) -> "started " + start.format(DateTimeFormatter.ofPattern("MMM d", Locale.getDefault()))
            else -> null
        }
    }

    fun priorityWords(p: String?) = when (p) { "urgent" -> "Urgent"; "high" -> "High"; "low" -> "Low"; else -> null }

    /** The second line of a row, in the board's words: Stuck and Ready to check in amber, the rest plain. */
    fun meta(t: TaskItem, today: LocalDate, now: LocalDateTime = LocalDateTime.now(ZoneOffset.UTC)): List<Run> {
        val sep = Run(" · ")
        return when (t.status) {
            "blocked" -> listOf(Run("Stuck", bold = true, amber = true)) + listOfNotNull(t.blockedReason?.takeIf { it.isNotBlank() }?.let { listOf(sep, Run(it)) }).flatten()
            "ready_for_review" -> listOf(Run("Ready to check", bold = true, amber = true)) + (readyAgo(t.readyAt, now)?.let { listOf(sep, Run("hq finished $it")) } ?: emptyList())
            else -> {
                val parts = mutableListOf<List<Run>>()
                priorityWords(t.priority)?.let { parts += listOf(Run(it, bold = true)) }
                dueWords(t.dueDate, today)?.let { parts += listOf(Run(it)) }
                parse(t.startDate)?.takeIf { it.isAfter(today) }?.let { parts += listOf(Run(if (it == today.plusDays(1)) "starts tomorrow" else "starts " + it.format(DateTimeFormatter.ofPattern("EEE", Locale.getDefault())))) }
                startedWords(t, today)?.let { parts += listOf(Run(it)) }
                if (t.subtaskCount > 0) parts += listOf(Run("${t.subtaskDone} of ${t.subtaskCount} steps done"))
                if (t.assignees.isNotEmpty() && t.status == "to_do") parts += listOf(Run("for ${t.assignees.first()}"))
                parts.flatMapIndexed { i, p -> if (i == 0) p else listOf(sep) + p }
            }
        }
    }

    /** "3 h ago" from the server's `YYYY-MM-DD HH:MM:SS` UTC time. */
    fun readyAgo(readyAt: String?, now: LocalDateTime): String? {
        val then = try { LocalDateTime.parse(readyAt?.replace(' ', 'T') ?: return null) } catch (e: DateTimeParseException) { return null }
        val minutes = Duration.between(then, now).toMinutes().coerceAtLeast(0)
        return when {
            minutes < 1 -> "just now"
            minutes < 60 -> "$minutes min ago"
            minutes < 24 * 60 -> "${minutes / 60} h ago"
            else -> "${minutes / (24 * 60)} d ago"
        }
    }

    fun statusWords(status: String) = when (status) {
        "to_do" -> "To do"; "in_progress" -> "In progress"; "blocked" -> "Stuck"; "ready_for_review" -> "Ready to check"; "complete" -> "Done"; else -> status
    }

    fun nextMonday(today: LocalDate): LocalDate = today.with(TemporalAdjusters.next(DayOfWeek.MONDAY))
}

/** What the one line of a new task means: the title, and the words HQ understood. */
data class QuickAdd(
    val title: String,
    val priority: String?,
    val startDate: LocalDate?,
    val initiative: String?,
    val understood: List<Understood>,
) {
    data class Understood(val label: String, val range: IntRange)

    companion object {
        private val WORDS = Regex("\\S+")

        /** Reads "tomorrow", "today", "next week", "!high", "!urgent", "!low" and "#name" wherever they stand; every other word stays in the title. */
        fun parse(text: String, today: LocalDate): QuickAdd {
            val tokens = WORDS.findAll(text).toList()
            var priority: String? = null
            var start: LocalDate? = null
            var initiative: String? = null
            val used = mutableSetOf<Int>()
            val understood = mutableListOf<Understood>()
            var i = 0
            while (i < tokens.size) {
                val w = tokens[i].value
                val lower = w.lowercase()
                when {
                    lower == "tomorrow" -> { start = today.plusDays(1); used += i; understood += Understood("Tomorrow", tokens[i].range) }
                    lower == "today" -> { start = today; used += i; understood += Understood("Today", tokens[i].range) }
                    lower == "next" && tokens.getOrNull(i + 1)?.value?.lowercase() == "week" -> {
                        start = TasksLogic.nextMonday(today); used += i; used += i + 1
                        understood += Understood("Next week", tokens[i].range.first..tokens[i + 1].range.last); i++
                    }
                    lower == "!high" || lower == "!urgent" || lower == "!low" -> { priority = lower.drop(1); used += i; understood += Understood(priority.replaceFirstChar { it.uppercase() }, tokens[i].range) }
                    w.length > 1 && w.startsWith("#") -> { initiative = w.drop(1); used += i; understood += Understood(w.drop(1).replaceFirstChar { it.uppercase() }, tokens[i].range) }
                }
                i++
            }
            val title = tokens.filterIndexed { idx, _ -> idx !in used }.joinToString(" ") { it.value }
            return QuickAdd(title, priority, start, initiative, understood)
        }
    }
}
