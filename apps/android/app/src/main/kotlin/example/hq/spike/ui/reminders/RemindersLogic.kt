package example.hq.spike.ui.reminders

import example.hq.spike.data.Reminder
import java.time.DayOfWeek
import java.time.Instant
import java.time.LocalTime
import java.time.ZoneId
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.time.temporal.ChronoUnit

/**
 * The times a reminder is set and snoozed to, worked out in the owner's own zone (the one HQ saved, else the phone's), and
 * how the list is grouped. Pure: each function takes the clock, so each is tested from a fixed instant. `java.time` puts a
 * time that a clock change skips just after the gap and takes the first of a time that happens twice, as HQ does.
 */
object RemindersLogic {
    const val EVENING_HOUR = 18
    const val MORNING_HOUR = 9
    const val SNOOZE_HOUR_MINUTES = 60L

    /** An evening choice that is nearly now is not offered: it would ring while the person is still looking. */
    private const val EVENING_MIN_LEAD_MINUTES = 30L

    enum class Choice { Hour, Evening, Tomorrow, Monday }

    data class Option(val choice: Choice, val at: ZonedDateTime)

    /** The zone a reminder's times mean: HQ's saved one when it names a real zone, else the phone's. */
    fun zoneOf(saved: String?, phone: ZoneId = ZoneId.systemDefault()): ZoneId =
        saved?.takeIf { it.isNotBlank() }?.let { runCatching { ZoneId.of(it) }.getOrNull() } ?: phone

    /** An hour from now, this evening (unless it is nearly 18:00), tomorrow at 09:00 and, if asked, next Monday at 09:00. */
    fun options(now: ZonedDateTime, monday: Boolean = false): List<Option> {
        val out = mutableListOf(Option(Choice.Hour, now.plusHours(1)))
        val evening = now.toLocalDate().atTime(LocalTime.of(EVENING_HOUR, 0)).atZone(now.zone)
        if (ChronoUnit.MINUTES.between(now, evening) >= EVENING_MIN_LEAD_MINUTES) out += Option(Choice.Evening, evening)
        out += Option(Choice.Tomorrow, tomorrowMorning(now))
        if (monday) out += Option(Choice.Monday, nextMonday(now))
        return out
    }

    fun tomorrowMorning(now: ZonedDateTime): ZonedDateTime =
        now.toLocalDate().plusDays(1).atTime(LocalTime.of(MORNING_HOUR, 0)).atZone(now.zone)

    /** The coming Monday at 09:00, never today. */
    fun nextMonday(now: ZonedDateTime): ZonedDateTime {
        val ahead = ((DayOfWeek.MONDAY.value - now.dayOfWeek.value + 7) % 7).let { if (it == 0) 7 else it }
        return now.toLocalDate().plusDays(ahead.toLong()).atTime(LocalTime.of(MORNING_HOUR, 0)).atZone(now.zone)
    }

    enum class RepeatChoice { None, Daily, Weekdays, Weekly, Monthly }

    /** The `repeat` HQ takes for a choice; weekly and monthly follow the day the reminder first rings. */
    fun repeatSpec(choice: RepeatChoice, first: ZonedDateTime): String? = when (choice) {
        RepeatChoice.None -> null
        RepeatChoice.Daily -> "daily"
        RepeatChoice.Weekdays -> "weekdays"
        RepeatChoice.Weekly -> "weekly:${first.dayOfWeek.value}"
        RepeatChoice.Monthly -> "monthly:${first.dayOfMonth}"
    }

    private val LOCAL: DateTimeFormatter = DateTimeFormatter.ofPattern("yyyy-MM-dd'T'HH:mm")

    /** `2026-11-27T16:00`: what HQ takes as `at`, with the zone sent beside it. */
    fun localString(at: ZonedDateTime): String = at.format(LOCAL)

    enum class Group { Overdue, Today, Tomorrow, Later }

    fun group(dueEpoch: Long, now: ZonedDateTime): Group {
        if (dueEpoch < now.toEpochSecond()) return Group.Overdue
        val ahead = ChronoUnit.DAYS.between(now.toLocalDate(), Instant.ofEpochSecond(dueEpoch).atZone(now.zone).toLocalDate())
        return when {
            ahead <= 0 -> Group.Today
            ahead == 1L -> Group.Tomorrow
            else -> Group.Later
        }
    }

    /** Overdue, today, tomorrow and later, each soonest first; a group with nothing in it is left out. */
    fun grouped(items: List<Reminder>, now: ZonedDateTime): List<Pair<Group, List<Reminder>>> {
        val sorted = items.sortedBy { it.dueEpoch }
        return Group.entries.map { g -> g to sorted.filter { group(it.dueEpoch, now) == g } }.filter { it.second.isNotEmpty() }
    }
}
