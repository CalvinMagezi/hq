package example.hq.spike.ui.notes

import java.time.Duration
import java.time.Instant
import java.time.OffsetDateTime
import java.time.format.DateTimeParseException

/** Ages for note lists. hq writes `+00:00` times, which Instant.parse cannot read before Android 14, so the offset form is read. */
object NoteTime {
    private const val MINUTES_PER_HOUR = 60L
    private const val HOURS_PER_DAY = 24L

    private fun parse(value: String): Instant? = try { OffsetDateTime.parse(value).toInstant() } catch (e: DateTimeParseException) { null }

    /** "now", "5m", "3h", "1d": the short form at the end of a list row. Blank when the time is not known. */
    fun short(mtime: String, now: Instant = Instant.now()): String {
        val then = parse(mtime) ?: return ""
        val minutes = Duration.between(then, now).toMinutes().coerceAtLeast(0)
        return when {
            minutes < 1 -> "now"
            minutes < MINUTES_PER_HOUR -> "${minutes}m"
            minutes < MINUTES_PER_HOUR * HOURS_PER_DAY -> "${minutes / MINUTES_PER_HOUR}h"
            else -> "${minutes / (MINUTES_PER_HOUR * HOURS_PER_DAY)}d"
        }
    }

    /** "edited 1d ago" for a note's top bar; "edited just now" under a minute. */
    fun edited(mtime: String, now: Instant = Instant.now()): String {
        val s = short(mtime, now)
        return when (s) { "" -> ""; "now" -> "edited just now"; else -> "edited $s ago" }
    }
}
