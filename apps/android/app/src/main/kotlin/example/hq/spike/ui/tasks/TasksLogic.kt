package example.hq.spike.ui.tasks

import example.hq.spike.ui.shell.Counts
import example.hq.spike.ui.shell.Words
import example.hq.spike.data.HqClient.TaskItem
import java.time.DayOfWeek
import java.time.Duration
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import example.hq.spike.R
import java.time.format.DateTimeParseException
import java.time.temporal.TemporalAdjusters
import java.util.Locale

enum class TaskTab { Today, Upcoming, All }

/** The lists on the All tab: what is open, the two states that need a person, what is finished, and what was put away. */
enum class AllChip {
    Active, Stuck, Review, Done, Archived;

    fun label(w: Words): String = w.s(when (this) {
        Active -> R.string.tasks_chip_active
        Stuck -> R.string.task_stuck
        Review -> R.string.tasks_status_ready
        Done -> R.string.tasks_status_complete
        Archived -> R.string.tasks_chip_archived
    })
}

/** The small tag on a row's first line: Urgent or High in amber, how long it has been stuck in red. */
data class IdTag(val text: String, val danger: Boolean)

/**
 * A heading and the tasks under it. The badge counts them; [collapsible] groups (Later, No date) show only their count until tapped;
 * a group with a [preview] shows that many and a row to show the rest.
 */
data class TaskGroup(val key: String, val title: String, val tasks: List<TaskItem>, val badge: Boolean = true, val collapsible: Boolean = false, val preview: Int? = null, val count: Int? = null)

/** One piece of a row's second line: some words, bold or not, plain or highlighted. */
data class Run(val text: String, val bold: Boolean = false, val amber: Boolean = false)

object TasksLogic {
    /** A task for the edge worker names its repository first, the way the worker looks for it; the owner's notes follow. */
    fun edgeDescription(repo: String?, notes: String): String {
        val text = notes.trim()
        if (repo == null) return text
        return if (text.isEmpty()) "Repository: $repo" else "Repository: $repo\n\n$text"
    }

    private const val SOON_DAYS = 7L
    const val WAITING_PREVIEW = 5
    /** A no-break space keeps "10 d ago" together when a row wraps. */
    private const val NBSP = '\u00A0'
    private val PRIORITY_ORDER = listOf("urgent", "high", "normal", "low")

    fun parse(date: String?): LocalDate? = try { date?.let(LocalDate::parse) } catch (e: DateTimeParseException) { null }

    internal fun open(t: TaskItem) = t.status != "complete" && t.parentId == null

    /** Priority first, then the oldest. */
    val byUrgency: Comparator<TaskItem> = compareBy<TaskItem>({ PRIORITY_ORDER.indexOf(it.priority).let { i -> if (i < 0) PRIORITY_ORDER.size else i } }, { it.updatedAt })

    /** The date a task belongs to in the lists: when it starts, else when it is due. */
    private fun when_(t: TaskItem): LocalDate? = parse(t.startDate) ?: parse(t.dueDate)

    fun today(w: Words, tasks: List<TaskItem>, today: LocalDate): List<TaskGroup> {
        val live = tasks.filter(::open)
        val waiting = live.filter { it.status == "blocked" || it.status == "ready_for_review" }.sortedWith(byUrgency)
        val going = live.filter { it.status == "in_progress" }.sortedWith(byUrgency)
        val due = live.filter { it.status == "to_do" }.filter { t -> listOfNotNull(parse(t.startDate), parse(t.dueDate)).any { !it.isAfter(today) } }.sortedWith(byUrgency)
        return listOf(
            TaskGroup("waiting", w.s(R.string.tasks_group_waiting), waiting, preview = WAITING_PREVIEW), TaskGroup("going", statusWords(w, "in_progress"), going), TaskGroup("due", w.s(R.string.tasks_group_due), due),
        ).filter { it.tasks.isNotEmpty() }
    }

    fun upcoming(w: Words, tasks: List<TaskItem>, today: LocalDate): List<TaskGroup> {
        val live = tasks.filter(::open).filter { it.status == "to_do" || it.status == "in_progress" }
        val dated = live.mapNotNull { t -> when_(t)?.takeIf { it.isAfter(today) }?.let { it to t } }
        val soon = dated.filter { (d, _) -> !d.isAfter(today.plusDays(SOON_DAYS)) }.groupBy({ it.first }, { it.second }).toSortedMap()
        val groups = soon.map { (d, ts) -> TaskGroup("d-$d", dayLabel(w, d, today), ts.sortedWith(byUrgency), badge = false) }.toMutableList()
        val later = dated.filter { (d, _) -> d.isAfter(today.plusDays(SOON_DAYS)) }.map { it.second }.sortedWith(byUrgency)
        if (later.isNotEmpty()) groups += TaskGroup("later", w.s(R.string.tasks_group_later), later, badge = false, collapsible = true, count = later.size)
        val undated = live.filter { when_(it) == null && it.status == "to_do" }.sortedWith(byUrgency)
        if (undated.isNotEmpty()) groups += TaskGroup("nodate", w.s(R.string.tasks_group_no_date), undated, badge = false, collapsible = true, count = undated.size)
        return groups
    }

    fun all(w: Words, tasks: List<TaskItem>): List<TaskGroup> {
        val live = tasks.filter(::open)
        return listOf(
            TaskGroup("going", statusWords(w, "in_progress"), live.filter { it.status == "in_progress" }.sortedWith(byUrgency)),
            TaskGroup("blocked", statusWords(w, "blocked"), live.filter { it.status == "blocked" }.sortedWith(byUrgency)),
            TaskGroup("review", statusWords(w, "ready_for_review"), live.filter { it.status == "ready_for_review" }.sortedWith(byUrgency)),
            TaskGroup("todo", statusWords(w, "to_do"), live.filter { it.status == "to_do" }.sortedWith(byUrgency)),
        ).filter { it.tasks.isNotEmpty() }
    }

    /** What the All tab shows under [chip]. Done and Archived are one list with no headings. */
    fun allList(w: Words, tasks: List<TaskItem>, chip: AllChip, archived: List<TaskItem>): List<TaskGroup> = when (chip) {
        AllChip.Active -> all(w, tasks)
        AllChip.Stuck -> all(w, tasks).filter { it.key == "blocked" }
        AllChip.Review -> all(w, tasks).filter { it.key == "review" }
        AllChip.Done -> flat("done", tasks.filter { it.status == "complete" && it.parentId == null }.sortedByDescending { it.completedAt ?: it.updatedAt })
        AllChip.Archived -> flat("archived", archived.sortedByDescending { it.archivedAt ?: it.updatedAt })
    }

    private fun flat(key: String, tasks: List<TaskItem>) = if (tasks.isEmpty()) emptyList() else listOf(TaskGroup(key, "", tasks, badge = false))

    /** The number on a chip: only the two that ask something of the owner carry one. */
    fun chipCount(tasks: List<TaskItem>, chip: AllChip): Int = when (chip) {
        AllChip.Stuck -> tasks.count { open(it) && it.status == "blocked" }
        AllChip.Review -> tasks.count { open(it) && it.status == "ready_for_review" }
        else -> 0
    }

    fun emptyAll(w: Words, chip: AllChip) = w.s(when (chip) {
        AllChip.Active -> R.string.tasks_empty_active; AllChip.Stuck -> R.string.tasks_empty_stuck; AllChip.Review -> R.string.tasks_empty_review
        AllChip.Done -> R.string.tasks_empty_done; AllChip.Archived -> R.string.tasks_empty_archived
    })

    /** "Blocked 2d" while stuck, else the priority when it is Urgent or High. */
    fun idTag(w: Words, t: TaskItem, now: LocalDateTime = LocalDateTime.now(ZoneOffset.UTC)): IdTag? {
        if (t.status == "blocked") return IdTag(stuckFor(w, t.blockedSince, now)?.let { w.s(R.string.tasks_blocked_for, it) } ?: w.s(R.string.tasks_blocked), danger = true)
        return priorityWords(w, t.priority)?.takeIf { t.priority == "urgent" || t.priority == "high" }?.let { IdTag(it, danger = false) }
    }

    /** The sub-tasks line: "None yet" with no sub-tasks, else "1 of 3 done". */
    fun subtaskLabel(w: Words, done: Int, total: Int): String = if (total == 0) w.s(R.string.tasks_none_yet) else Counts.progress(w, done, total, subTasks = false)

    /** "2d", or "5h" within a day, from the server's `YYYY-MM-DD HH:MM:SS` UTC time. */
    fun stuckFor(w: Words, since: String?, now: LocalDateTime): String? {
        val then = example.hq.spike.data.Instants.parseUtc(since) ?: return null
        val hours = Duration.between(then, now).toHours().coerceAtLeast(0)
        return if (hours >= 24) w.s(R.string.tasks_stuck_days, (hours / 24).toString()) else if (hours < 1) w.s(R.string.tasks_stuck_under_hour) else w.s(R.string.tasks_stuck_hours, hours.toString())
    }

    /** "Claude, on T-4" for one helper, "Claude on T-4, and 1 more" for several; the task shows as its board number. */
    fun stripLine(w: Words, sessions: List<example.hq.spike.data.HqClient.WorkSession>, tasks: List<TaskItem>): String {
        val first = sessions.first()
        val helper = example.hq.spike.ui.helpers.HelperWords.agentName(w, first.harness.ifBlank { first.actor })
        val task = tasks.firstOrNull { it.id == first.taskId }?.displayId ?: first.cwd.trimEnd('/').substringAfterLast('/').ifBlank { first.taskId }
        return if (sessions.size == 1) w.s(R.string.tasks_strip_first, helper, task) else w.s(R.string.tasks_strip_more, helper, task, sessions.size - 1)
    }

    /** The third line of an All row: what it is waiting on, who it is for, how far along, and when. */
    fun allMeta(w: Words, t: TaskItem, today: LocalDate, working: Boolean, now: LocalDateTime = LocalDateTime.now(ZoneOffset.UTC)): String {
        val forWho = t.assignees.firstOrNull()?.let { if (it.equals("hq", ignoreCase = true)) w.s(R.string.tasks_for_hq) else w.s(R.string.tasks_for_who, it) }
        val tag = t.tags.firstOrNull()
        val parts = when {
            t.archivedAt != null -> listOfNotNull(w.s(R.string.tasks_archived_ago, readyAgo(w, t.archivedAt, now) ?: w.s(R.string.tasks_earlier)))
            t.status == "complete" -> listOfNotNull(w.s(R.string.tasks_done_ago, readyAgo(w, t.completedAt ?: t.updatedAt, now) ?: w.s(R.string.tasks_earlier)))
            t.status == "blocked" -> listOfNotNull(t.waitingOn?.takeIf { it.isNotBlank() }?.let { w.s(R.string.tasks_waiting_on, it) }, t.blockedReason?.takeIf { it.isNotBlank() })
            t.status == "ready_for_review" -> listOfNotNull(forWho, readyAgo(w, t.readyAt, now)?.let { w.s(R.string.tasks_moved_ago, it) })
            else -> listOfNotNull(
                w.s(R.string.tasks_hq_working).takeIf { working },
                Counts.progress(w, t.subtaskDone, t.subtaskCount).takeIf { t.subtaskCount > 0 },
                dueWords(w, t.dueDate, today), forWho, tag,
            )
        }
        return parts.joinToString(" \u00b7 ")
    }

    private fun dayLabel(w: Words, d: LocalDate, today: LocalDate): String {
        val long = d.format(DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM))
        return if (d == today.plusDays(1)) w.s(R.string.tasks_tomorrow_day, long) else long
    }

    /** "due today", "due tomorrow", "due Thursday" within a week, else "due Oct 24". */
    fun dueWords(w: Words, due: String?, today: LocalDate): String? {
        val d = parse(due) ?: return null
        return when {
            d == today -> w.s(R.string.tasks_due_today)
            d == today.plusDays(1) -> w.s(R.string.tasks_due_tomorrow)
            d.isBefore(today) -> w.s(R.string.tasks_overdue_since, d.format(DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM)))
            !d.isAfter(today.plusDays(SOON_DAYS - 1)) -> w.s(R.string.tasks_due_on, d.format(DateTimeFormatter.ofPattern("EEEE", Locale.getDefault())))
            else -> w.s(R.string.tasks_due_on, d.format(DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM)))
        }
    }

    /** Why a to-do is on the Today list when it is not due today: it starts today, or it started earlier and is still open. */
    fun startedWords(w: Words, t: TaskItem, today: LocalDate): String? {
        val start = parse(t.startDate) ?: return null
        val due = parse(t.dueDate)
        // A task that is already due or overdue says so in its due words; "started" would only repeat it.
        if (t.status != "to_do" || (due != null && !due.isAfter(today))) return null
        return when {
            start == today -> w.s(R.string.tasks_starts_today)
            start.isBefore(today) -> w.s(R.string.tasks_started_on, start.format(DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM)))
            else -> null
        }
    }

    fun priorityWords(w: Words, p: String?): String? = when (p) {
        "urgent" -> w.s(R.string.tasks_priority_urgent); "high" -> w.s(R.string.tasks_priority_high); "low" -> w.s(R.string.tasks_priority_low); else -> null
    }

    /** The second line of a row, in the board's words: Stuck and Ready to check in amber, the rest plain. */
    fun meta(w: Words, t: TaskItem, today: LocalDate, now: LocalDateTime = LocalDateTime.now(ZoneOffset.UTC)): List<Run> {
        val sep = Run(" · ")
        return when (t.status) {
            "blocked" -> listOf(Run(statusWords(w, "blocked"), bold = true, amber = true)) + listOfNotNull(t.blockedReason?.takeIf { it.isNotBlank() }?.let { listOf(sep, Run(it)) }).flatten()
            "ready_for_review" -> listOf(Run(statusWords(w, "ready_for_review"), bold = true, amber = true)) + (readyAgo(w, t.readyAt, now)?.let { listOf(sep, Run(w.s(R.string.tasks_hq_finished, it.replace(' ', NBSP)))) } ?: emptyList())
            else -> {
                val parts = mutableListOf<List<Run>>()
                priorityWords(w, t.priority)?.let { parts += listOf(Run(it, bold = true)) }
                dueWords(w, t.dueDate, today)?.let { parts += listOf(Run(it)) }
                parse(t.startDate)?.takeIf { it.isAfter(today) }?.let { parts += listOf(Run(if (it == today.plusDays(1)) w.s(R.string.tasks_starts_tomorrow) else w.s(R.string.tasks_starts_on, it.format(DateTimeFormatter.ofPattern("EEE", Locale.getDefault()))))) }
                startedWords(w, t, today)?.let { parts += listOf(Run(it)) }
                if (t.subtaskCount > 0) parts += listOf(Run(Counts.progress(w, t.subtaskDone, t.subtaskCount)))
                if (t.assignees.isNotEmpty() && t.status == "to_do") parts += listOf(Run(w.s(R.string.tasks_for_who, t.assignees.first())))
                parts.flatMapIndexed { i, p -> if (i == 0) p else listOf(sep) + p }
            }
        }
    }

    /** "3 h ago" from the server's `YYYY-MM-DD HH:MM:SS` UTC time. */
    fun readyAgo(w: Words, readyAt: String?, now: LocalDateTime): String? {
        val then = example.hq.spike.data.Instants.parseUtc(readyAt) ?: return null
        val minutes = Duration.between(then, now).toMinutes().coerceAtLeast(0)
        return when {
            minutes < 1 -> w.s(R.string.age_just_now)
            minutes < 60 -> w.s(R.string.time_min_ago, minutes.toString())
            minutes < 24 * 60 -> w.s(R.string.age_hours, (minutes / 60).toString())
            else -> w.s(R.string.age_days, (minutes / (24 * 60)).toString())
        }
    }

    fun statusWords(w: Words, status: String): String = when (status) {
        "to_do" -> w.s(R.string.tasks_status_to_do); "in_progress" -> w.s(R.string.tasks_status_in_progress); "blocked" -> w.s(R.string.task_stuck)
        "ready_for_review" -> w.s(R.string.tasks_status_ready); "complete" -> w.s(R.string.tasks_status_complete); else -> status
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
        fun parse(w: Words, text: String, today: LocalDate): QuickAdd {
            val tokens = WORDS.findAll(text).toList()
            var priority: String? = null
            var start: LocalDate? = null
            var initiative: String? = null
            val used = mutableSetOf<Int>()
            val understood = mutableListOf<Understood>()
            var i = 0
            while (i < tokens.size) {
                val word = tokens[i].value
                val lower = word.lowercase()
                when {
                    lower == "tomorrow" -> { start = today.plusDays(1); used += i; understood += Understood(w.s(R.string.tasks_tomorrow), tokens[i].range) }
                    lower == "today" -> { start = today; used += i; understood += Understood(w.s(R.string.tasks_today), tokens[i].range) }
                    lower == "next" && tokens.getOrNull(i + 1)?.value?.lowercase() == "week" -> {
                        start = TasksLogic.nextMonday(today); used += i; used += i + 1
                        understood += Understood(w.s(R.string.tasks_next_week), tokens[i].range.first..tokens[i + 1].range.last); i++
                    }
                    lower == "!high" || lower == "!urgent" || lower == "!low" -> { priority = lower.drop(1); used += i; understood += Understood(TasksLogic.priorityWords(w, priority) ?: priority, tokens[i].range) }
                    word.length > 1 && word.startsWith("#") -> { initiative = word.drop(1); used += i; understood += Understood(word.drop(1).replaceFirstChar { it.uppercase() }, tokens[i].range) }
                }
                i++
            }
            val title = tokens.filterIndexed { idx, _ -> idx !in used }.joinToString(" ") { it.value }
            return QuickAdd(title, priority, start, initiative, understood)
        }
    }
}
