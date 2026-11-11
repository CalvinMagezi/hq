package example.hq.spike.calendar

import org.json.JSONArray
import org.json.JSONObject
import java.time.DateTimeException
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.ZoneOffset

/** Bounds of the list contract; HQ checks them too, the phone does not trust that. */
object CalendarListLimits {
    const val MAX_EVENTS = 50
    const val WINDOW_DAYS = 30L
    const val WINDOW_MS = WINDOW_DAYS * 24 * 60 * 60 * 1000
    const val NEXT_STEP_PERMISSION = "Ask the owner to allow calendar access in HQ's phone settings, under Calendar."
    const val MISSING_READ = "read_calendar"
    const val REASON_PROVIDER_ERROR = "provider_error"
}

/** The only fields of an event that leave the provider read. Never logged. */
data class CalendarInstance(
    val eventId: Long,
    val title: String,
    val beginMs: Long,
    val endMs: Long,
    val allDay: Boolean,
    val calendarLabel: String,
    val visible: Boolean,
)

/** The Calendar Provider's Instances table behind a seam, so window walking runs on the JVM against a fake. */
interface CalendarReader {
    fun hasPermission(): Boolean

    /** Instances overlapping [fromMs, toMs), any order. May throw [CalendarStoreException]. */
    fun instances(fromMs: Long, toMs: Long): List<CalendarInstance>
}

data class CalendarListRequest(val requestId: String, val fromMs: Long, val toMs: Long, val zone: String, val limit: Int)

sealed interface ParsedListRequest {
    data class Ok(val request: CalendarListRequest) : ParsedListRequest
    data class Bad(val outcome: CalendarOutcome.InvalidEvent) : ParsedListRequest
}

object CalendarListRequests {
    private val KNOWN = setOf("request_id", "from_ms", "to_ms", "zone", "limit")

    fun parse(payload: JSONObject): ParsedListRequest {
        val unknown = payload.keys().asSequence().firstOrNull { it !in KNOWN }
        if (unknown != null) return bad("unknown_field", unknown)
        val requestId = payload.optString("request_id")
        if (!CalendarRequests.isUuid(requestId)) return bad("invalid_request_id", "request_id")
        val from = payload.optLong("from_ms", -1L)
        val to = payload.optLong("to_ms", -1L)
        if (!payload.has("from_ms") || !payload.has("to_ms") || from < 0 || to <= from) return bad("invalid_range", "to_ms")
        val zone = payload.optString("zone")
        try { ZoneId.of(zone) } catch (e: DateTimeException) { return bad("invalid_time_zone", "zone") }
        val limit = payload.optInt("limit", CalendarListLimits.MAX_EVENTS)
        if (limit < 1) return bad("invalid_limit", "limit")
        return ParsedListRequest.Ok(CalendarListRequest(requestId, from, to, zone, minOf(limit, CalendarListLimits.MAX_EVENTS)))
    }

    private fun bad(reason: String, field: String) = ParsedListRequest.Bad(CalendarOutcome.InvalidEvent(reason, field))
}

/** One page of events, in order, with where the next page starts when there was more. */
data class CalendarEventRef(val id: Long, val beginMs: Long, val endMs: Long, val allDay: Boolean, val title: String, val calendarLabel: String)

data class Listed(val events: List<CalendarEventRef>, val truncated: Boolean, val nextFromMs: Long?) : CalendarOutcome {
    override val code get() = "listed"
    override fun fill(o: JSONObject) {
        val rows = JSONArray()
        events.forEach {
            rows.put(JSONObject().put("id", it.id).put("begin_ms", it.beginMs).put("end_ms", it.endMs).put("all_day", it.allDay).put("title", it.title).put("calendar_label", it.calendarLabel))
        }
        o.put("events", rows).put("truncated", truncated).put("next_from_ms", nextFromMs ?: JSONObject.NULL)
    }
}

/** The app-side steps of one calendar.list. Pure apart from the reader. */
class CalendarLister(private val reader: CalendarReader) {

    fun list(request: CalendarListRequest): CalendarOutcome {
        if (!reader.hasPermission()) return CalendarOutcome.PermissionDenied(listOf(CalendarListLimits.MISSING_READ), CalendarListLimits.NEXT_STEP_PERMISSION)
        val range = RequestRange.of(request)
        val found = ArrayList<CalendarInstance>()
        val seen = HashSet<Pair<Long, Long>>()
        // One more than the limit tells whether the answer is cut, and where the next page begins.
        val want = request.limit + 1
        try {
            var windowStart = request.fromMs
            while (windowStart < request.toMs && found.size < want) {
                val windowEnd = minOf(request.toMs, windowStart + CalendarListLimits.WINDOW_MS)
                reader.instances(windowStart, windowEnd)
                    .filter { it.visible && it.endMs >= it.beginMs && inWindow(it, range) }
                    .sortedWith(compareBy({ it.beginMs }, { it.eventId }))
                    .forEach { if (found.size < want && seen.add(it.eventId to it.beginMs)) found += it }
                windowStart = windowEnd
            }
        } catch (e: CalendarStoreException) {
            return CalendarOutcome.Unavailable(CalendarListLimits.REASON_PROVIDER_ERROR)
        }
        val page = found.take(request.limit)
        val next = found.getOrNull(request.limit)
        return Listed(page.map { CalendarEventRef(it.eventId, it.beginMs, it.endMs, it.allDay, it.title, it.calendarLabel) }, next != null, next?.beginMs)
    }

    /** The request window as instants and as the phone's calendar dates (end exclusive). */
    private class RequestRange(val fromMs: Long, val toMs: Long, val startDate: LocalDate, val endDateExclusive: LocalDate) {
        companion object {
            fun of(r: CalendarListRequest): RequestRange {
                val zone = ZoneId.of(r.zone)
                val start = Instant.ofEpochMilli(r.fromMs).atZone(zone).toLocalDate()
                val end = Instant.ofEpochMilli(r.toMs - 1).atZone(zone).toLocalDate().plusDays(1)
                return RequestRange(r.fromMs, r.toMs, start, end)
            }
        }
    }

    /** All-day rows are UTC midnights, so they are judged by date; instants would pull in the neighbouring day for any zone but UTC. */
    private fun inWindow(i: CalendarInstance, r: RequestRange): Boolean {
        if (i.allDay) {
            val first = Instant.ofEpochMilli(i.beginMs).atOffset(ZoneOffset.UTC).toLocalDate()
            val endExclusive = Instant.ofEpochMilli(i.endMs).atOffset(ZoneOffset.UTC).toLocalDate()
            return first < r.endDateExclusive && endExclusive > r.startDate
        }
        if (i.endMs == i.beginMs) return i.beginMs >= r.fromMs && i.beginMs < r.toMs
        return i.beginMs < r.toMs && i.endMs > r.fromMs
    }
}
