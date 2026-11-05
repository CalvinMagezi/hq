package example.hq.spike.data

import java.time.Instant
import java.time.LocalDateTime
import java.time.OffsetDateTime
import java.time.ZoneOffset
import java.time.format.DateTimeParseException

/** The one reader for a time HQ sends. Instant.parse refuses `+00:00` before Android 14, so OffsetDateTime does the work. */
object Instants {
    /** RFC 3339 with `Z` or an offset, with or without fractions, or the older `YYYY-MM-DD HH:MM:SS` (taken as UTC). Null when unreadable. */
    fun parse(text: String?): Instant? {
        val t = text?.trim()?.replace(' ', 'T')?.takeIf { it.isNotEmpty() } ?: return null
        return try { OffsetDateTime.parse(t).toInstant() } catch (e: DateTimeParseException) {
            try { LocalDateTime.parse(t).toInstant(ZoneOffset.UTC) } catch (e2: DateTimeParseException) { null }
        }
    }

    fun parseUtc(text: String?): LocalDateTime? = parse(text)?.let { LocalDateTime.ofInstant(it, ZoneOffset.UTC) }
}
