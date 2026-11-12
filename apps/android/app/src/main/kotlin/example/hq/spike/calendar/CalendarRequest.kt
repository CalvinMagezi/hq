package example.hq.spike.calendar

import java.time.DateTimeException
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.format.DateTimeParseException
import java.util.UUID
import example.hq.spike.deviceactions.DeviceOutcome
import org.json.JSONArray
import org.json.JSONObject

/** Bounds from the device action contract; the app re-checks them because HQ's check is not the phone's to trust. */
object CalendarLimits {
    const val TITLE_MAX = 200
    const val LOCATION_MAX = 200
    const val DESCRIPTION_MAX = 2000
    const val DURATION_MIN = 1
    const val DURATION_MAX = 1440
    const val MINUTES_PER_DAY = 1440L
    const val MS_PER_MINUTE = 60_000L
    const val WIRE_VERSION = 1
    const val ACTION_NAME = "calendar.create"
    const val LIST_ACTION_NAME = "calendar.list"
}

/** A moment the provider stores: epoch millis in UTC plus the zone the row is saved with. */
data class CalendarSpan(val startMs: Long, val endMs: Long, val timeZone: String)

/** One calendar-create request, already checked. */
data class CalendarRequest(
    val requestId: String,
    val title: String,
    val startLocal: String,
    val endLocal: String,
    val span: CalendarSpan,
    val allDay: Boolean,
    val location: String?,
    val description: String?,
    val durationDefaulted: Boolean,
)

sealed interface ParsedRequest {
    data class Ok(val request: CalendarRequest) : ParsedRequest
    data class Bad(val outcome: CalendarOutcome.InvalidEvent) : ParsedRequest
}

/** What the phone reports back; the shapes are the contract's, so HQ can pass them on unchanged. */
sealed interface CalendarOutcome : DeviceOutcome {
    override val code: String
    override fun toJson(): JSONObject = JSONObject().put("v", CalendarLimits.WIRE_VERSION).put("outcome", code).also { fill(it) }
    fun fill(o: JSONObject)

    data class Created(val eventRef: String, val title: String, val startLocal: String, val endLocal: String, val timeZone: String, val calendarLabel: String, val durationDefaulted: Boolean) : CalendarOutcome {
        override val code get() = "created"
        override fun fill(o: JSONObject) {
            o.put("event_ref", eventRef).put("title", title).put("start_local", startLocal).put("end_local", endLocal)
                .put("time_zone", timeZone).put("calendar_label", calendarLabel).put("duration_defaulted", durationDefaulted)
                .put("saved_to", "device_calendar")
        }
    }

    data class Clarification(val reason: String, val ask: String, val choices: List<String> = emptyList()) : CalendarOutcome {
        override val code get() = "clarification_required"
        override fun fill(o: JSONObject) {
            o.put("reason", reason).put("ask", ask)
            if (choices.isNotEmpty()) o.put("choices", JSONArray(choices))
        }
    }

    data class PermissionDenied(val missing: List<String>, val nextStep: String) : CalendarOutcome {
        override val code get() = "permission_denied"
        override fun fill(o: JSONObject) { o.put("missing", JSONArray(missing)).put("next_step", nextStep) }
    }

    data class InvalidEvent(val reason: String, val field: String? = null) : CalendarOutcome {
        override val code get() = "invalid_event"
        override fun fill(o: JSONObject) { o.put("reason", reason); field?.let { o.put("field", it) } }
    }

    data class Unavailable(val reason: String) : CalendarOutcome {
        override val code get() = "calendar_unavailable"
        override fun fill(o: JSONObject) { o.put("reason", reason) }
    }

    data class Unverified(val reason: String) : CalendarOutcome {
        override val code get() = "creation_unverified"
        override fun fill(o: JSONObject) { o.put("reason", reason) }
    }
}

/** Reading and checking the `payload` of a `device_action` frame. Pure, so every bound is a unit test. */
object CalendarRequests {
    private val KNOWN = setOf("request_id", "title", "start_local", "time_zone", "end_local", "duration_minutes", "all_day", "location", "description", "duration_defaulted")

    fun parse(payload: JSONObject): ParsedRequest {
        val unknown = payload.keys().asSequence().firstOrNull { it !in KNOWN }
        if (unknown != null) return bad("unknown_field", unknown)
        val requestId = payload.optString("request_id")
        if (!isUuid(requestId)) return bad("invalid_request_id", "request_id")
        val title = payload.optString("title").trim()
        if (title.isEmpty() || title.length > CalendarLimits.TITLE_MAX) return bad("title_length", "title")
        val location = payload.optNullable("location")
        if ((location?.length ?: 0) > CalendarLimits.LOCATION_MAX) return bad("location_too_long", "location")
        val description = payload.optNullable("description")
        if ((description?.length ?: 0) > CalendarLimits.DESCRIPTION_MAX) return bad("description_too_long", "description")
        val zone = try { ZoneId.of(payload.optString("time_zone")) } catch (e: DateTimeException) { return bad("invalid_time_zone", "time_zone") }
        val allDay = payload.optBoolean("all_day", false)
        val endLocal = payload.optNullable("end_local")
        val hasDuration = payload.has("duration_minutes") && !payload.isNull("duration_minutes")
        if (endLocal != null && hasDuration) return bad("end_and_duration", "duration_minutes")
        val duration = if (hasDuration) payload.optInt("duration_minutes", -1) else null
        if (duration != null && duration !in CalendarLimits.DURATION_MIN..CalendarLimits.DURATION_MAX) return bad("duration_range", "duration_minutes")
        val startLocal = payload.optString("start_local")
        val span = if (allDay) allDaySpan(startLocal, endLocal, duration) else timedSpan(startLocal, endLocal, duration, zone)
        return when (span) {
            is SpanResult.Bad -> Bad(span.outcome)
            is SpanResult.Ok -> Ok(CalendarRequest(requestId, title, startLocal, span.endLocal, span.span, allDay, location, description, payload.optBoolean("duration_defaulted", false)))
        }
    }

    private fun bad(reason: String, field: String?) = ParsedRequest.Bad(CalendarOutcome.InvalidEvent(reason, field))
    private fun Ok(r: CalendarRequest) = ParsedRequest.Ok(r)
    private fun Bad(o: CalendarOutcome.InvalidEvent) = ParsedRequest.Bad(o)

    internal fun isUuid(s: String) = try { UUID.fromString(s).toString() == s.lowercase() } catch (e: IllegalArgumentException) { false }

    private fun JSONObject.optNullable(key: String): String? = if (isNull(key)) null else optString(key).takeIf { it.isNotBlank() }

    private sealed interface SpanResult {
        data class Ok(val span: CalendarSpan, val endLocal: String) : SpanResult
        data class Bad(val outcome: CalendarOutcome.InvalidEvent) : SpanResult
    }

    private fun badSpan(reason: String, field: String) = SpanResult.Bad(CalendarOutcome.InvalidEvent(reason, field))

    /** All-day rows are UTC midnight to the next UTC midnight (end exclusive) whatever the phone's zone is. HQ's `end_local` is the last day, inclusive. */
    private fun allDaySpan(start: String, end: String?, durationMinutes: Int?): SpanResult {
        val first = try { LocalDate.parse(start) } catch (e: DateTimeParseException) { return badSpan("invalid_start", "start_local") }
        val lastDay = when {
            end != null -> try { LocalDate.parse(end) } catch (e: DateTimeParseException) { return badSpan("invalid_end", "end_local") }
            else -> first.plusDays(maxOf(1L, ceilDays(durationMinutes ?: CalendarLimits.DURATION_MIN)) - 1)
        }
        if (lastDay.isBefore(first)) return badSpan("end_not_after_start", "end_local")
        val exclusiveEnd = lastDay.plusDays(1)
        val span = CalendarSpan(first.atStartOfDay().toInstant(ZoneOffset.UTC).toEpochMilli(), exclusiveEnd.atStartOfDay().toInstant(ZoneOffset.UTC).toEpochMilli(), "UTC")
        return SpanResult.Ok(span, lastDay.toString())
    }

    private fun ceilDays(minutes: Int): Long = (minutes + CalendarLimits.MINUTES_PER_DAY - 1) / CalendarLimits.MINUTES_PER_DAY

    private fun timedSpan(start: String, end: String?, durationMinutes: Int?, zone: ZoneId): SpanResult {
        val s = try { LocalDateTime.parse(start) } catch (e: DateTimeParseException) { return badSpan("invalid_start", "start_local") }
        if (zone.rules.getValidOffsets(s).isEmpty()) return badSpan("nonexistent_local_time", "start_local")
        val e = if (end != null) {
            try { LocalDateTime.parse(end) } catch (x: DateTimeParseException) { return badSpan("invalid_end", "end_local") }
        } else {
            s.plusMinutes((durationMinutes ?: CalendarLimits.DURATION_MIN).toLong())
        }
        if (zone.rules.getValidOffsets(e).isEmpty()) return badSpan("nonexistent_local_time", "end_local")
        val startMs = s.atZone(zone).toInstant().toEpochMilli()
        val endMs = e.atZone(zone).toInstant().toEpochMilli()
        if (endMs <= startMs) return badSpan("end_not_after_start", "end_local")
        return SpanResult.Ok(CalendarSpan(startMs, endMs, zone.id), e.toString())
    }
}
