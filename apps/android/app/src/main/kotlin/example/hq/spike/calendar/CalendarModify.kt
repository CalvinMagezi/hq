package example.hq.spike.calendar

import java.time.DateTimeException
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.ZoneOffset
import org.json.JSONObject

object CalendarModifyLimits {
    const val UPDATE_ACTION_NAME = "calendar.update"
    const val DELETE_ACTION_NAME = "calendar.delete"
    const val DAY_MS = 24 * 60 * 60 * 1000L

    /** CalendarContract.Calendars.CAL_ACCESS_EDITOR; a plain constant keeps the JVM tests free of the Android framework. */
    const val ACCESS_EDITOR = 600
    const val CLEAN_MAX = 200
    const val REASON_PROVIDER_ERROR = "provider_error"
    const val REASON_UPDATE_REFUSED = "update_refused"
    const val REASON_DELETE_REFUSED = "delete_refused"
    const val REASON_EVENT_MISSING = "event_missing"
    const val REASON_READ_FAILED = "read_back_failed"
    const val REASON_MISMATCH = "read_back_mismatch"
    const val REASON_STILL_PRESENT = "still_present"
}

/** One line, no control characters, at most [max] characters: the same rule HQ applies to phone strings, so both sides compare alike. */
fun cleanPhoneText(raw: String, max: Int = CalendarModifyLimits.CLEAN_MAX): String =
    raw.map { if (it.isISOControl()) ' ' else it }
        .joinToString("")
        .split(Regex("[\\p{javaWhitespace}\\p{javaSpaceChar}]+"))
        .filter { it.isNotEmpty() }
        .joinToString(" ")
        .take(max)

/** The columns of one provider row that the edit checks need, plus its calendar's access level. Never logged. */
data class EventRow(
    val calendarId: Long,
    val title: String,
    val startMs: Long,
    val endMs: Long,
    val allDay: Boolean,
    val rrule: String,
    val rdate: String,
    val originalId: String,
    val originalInstanceTime: Long?,
    val deleted: Boolean,
    val accessLevel: Int,
) {
    val recurring: Boolean get() = rrule.isNotBlank() || rdate.isNotBlank() || originalId.isNotBlank() || originalInstanceTime != null
}

/** The columns an update writes; null means the column is left alone. */
data class EventWrite(val title: String?, val startMs: Long?, val endMs: Long?)

/** The Calendar Provider's edit side behind a seam, so the steps run on the JVM against a fake. */
interface CalendarEditor {
    /** Both calendar permissions, since an edit reads and writes. */
    fun hasPermission(): Boolean

    /** Null when no row has this id. A deleted row is returned with `deleted` set. May throw [CalendarStoreException]. */
    fun row(eventId: Long): EventRow?

    /** False when the provider changed no row. May throw [CalendarStoreException]. */
    fun update(eventId: Long, write: EventWrite): Boolean

    /** False when the provider removed no row. May throw [CalendarStoreException]. */
    fun delete(eventId: Long): Boolean
}

/** One calendar.update or calendar.delete request, already checked. [changes] is null for a delete. */
data class ModifyRequest(val requestId: String, val eventId: Long, val beginMs: Long, val expectedTitle: String, val zone: String, val changes: EventWrite?)

sealed interface ParsedModify {
    data class Ok(val request: ModifyRequest) : ParsedModify
    data class Bad(val outcome: CalendarOutcome.InvalidEvent) : ParsedModify
}

object CalendarModifyRequests {
    private val KNOWN_DELETE = setOf("request_id", "event_id", "begin_ms", "expected_title", "zone")
    private val KNOWN_CHANGES = setOf("title", "start_ms", "end_ms")

    fun parseDelete(payload: JSONObject): ParsedModify = parse(payload, withChanges = false)

    fun parseUpdate(payload: JSONObject): ParsedModify = parse(payload, withChanges = true)

    private fun parse(payload: JSONObject, withChanges: Boolean): ParsedModify {
        val known = if (withChanges) KNOWN_DELETE + "changes" else KNOWN_DELETE
        val unknown = payload.keys().asSequence().firstOrNull { it !in known }
        if (unknown != null) return bad("unknown_field", unknown)
        val requestId = payload.optString("request_id")
        if (!CalendarRequests.isUuid(requestId)) return bad("invalid_request_id", "request_id")
        val eventId = payload.optLong("event_id", -1L)
        if (!payload.has("event_id") || eventId <= 0) return bad("invalid_event_id", "event_id")
        val beginMs = payload.optLong("begin_ms", -1L)
        if (!payload.has("begin_ms") || beginMs < 0) return bad("invalid_begin", "begin_ms")
        if (!payload.has("expected_title") || payload.isNull("expected_title")) return bad("title_missing", "expected_title")
        val zone = payload.optString("zone")
        try { ZoneId.of(zone) } catch (e: DateTimeException) { return bad("invalid_time_zone", "zone") }
        var changes: EventWrite? = null
        if (withChanges) {
            when (val c = parseChanges(payload.optJSONObject("changes"))) {
                is ParsedChanges.Bad -> return c.bad
                is ParsedChanges.Ok -> changes = c.write
            }
        }
        return ParsedModify.Ok(ModifyRequest(requestId, eventId, beginMs, payload.optString("expected_title"), zone, changes))
    }

    private sealed interface ParsedChanges {
        class Ok(val write: EventWrite) : ParsedChanges
        class Bad(val bad: ParsedModify.Bad) : ParsedChanges
    }

    private fun parseChanges(raw: JSONObject?): ParsedChanges {
        if (raw == null) return ParsedChanges.Bad(bad("changes_missing", "changes"))
        val unknown = raw.keys().asSequence().firstOrNull { it !in KNOWN_CHANGES }
        if (unknown != null) return ParsedChanges.Bad(bad("unknown_field", unknown))
        val title = if (raw.has("title")) raw.optString("title").trim() else null
        if (title != null && (title.isEmpty() || title.length > CalendarLimits.TITLE_MAX)) return ParsedChanges.Bad(bad("title_length", "title"))
        val start = timeChange(raw, "start_ms") ?: return ParsedChanges.Bad(bad("invalid_start_ms", "start_ms"))
        val end = timeChange(raw, "end_ms") ?: return ParsedChanges.Bad(bad("invalid_end_ms", "end_ms"))
        if (title == null && start.value == null && end.value == null) return ParsedChanges.Bad(bad("no_changes", "changes"))
        return ParsedChanges.Ok(EventWrite(title, start.value, end.value))
    }

    /** Absent is a valid "no change" (a holder with a null value); a negative or non-numeric value is null, the invalid case. */
    private class TimeChange(val value: Long?)

    private fun timeChange(raw: JSONObject, key: String): TimeChange? {
        if (!raw.has(key)) return TimeChange(null)
        val v = raw.optLong(key, -1L)
        return if (v < 0) null else TimeChange(v)
    }

    private fun bad(reason: String, field: String) = ParsedModify.Bad(CalendarOutcome.InvalidEvent(reason, field))
}

data class Updated(val eventRef: String) : CalendarOutcome {
    override val code get() = "updated"
    override fun fill(o: JSONObject) { o.put("event_ref", eventRef) }
}

data class Deleted(val eventRef: String) : CalendarOutcome {
    override val code get() = "deleted"
    override fun fill(o: JSONObject) { o.put("event_ref", eventRef) }
}

object EventNotFound : CalendarOutcome {
    override val code get() = "event_not_found"
    override fun fill(o: JSONObject) {}
}

object EventChanged : CalendarOutcome {
    override val code get() = "event_changed"
    override fun fill(o: JSONObject) {}
}

object ReadOnlyCalendar : CalendarOutcome {
    override val code get() = "read_only_calendar"
    override fun fill(o: JSONObject) {}
}

object RecurringUnsupported : CalendarOutcome {
    override val code get() = "recurring_unsupported"
    override fun fill(o: JSONObject) {}
}

data class UpdateUnverified(val reason: String) : CalendarOutcome {
    override val code get() = "update_unverified"
    override fun fill(o: JSONObject) { o.put("reason", reason) }
}

data class DeleteUnverified(val reason: String) : CalendarOutcome {
    override val code get() = "delete_unverified"
    override fun fill(o: JSONObject) { o.put("reason", reason) }
}

/** The app-side steps of one calendar.update or calendar.delete. Pure apart from the editor and the ledger. */
class CalendarModifier(private val editor: CalendarEditor, private val ledger: CalendarLedger) {

    fun update(request: ModifyRequest): CalendarOutcome {
        val changes = request.changes ?: return CalendarOutcome.InvalidEvent("no_changes", "changes")
        if (!editor.hasPermission()) return permissionDenied()
        ledger.get(request.requestId)?.let { return replayUpdate(it, changes) }
        val row = when (val g = gate(request)) {
            is Gate.Stop -> return g.outcome
            is Gate.Go -> g.row
        }
        val write = when (val p = plan(row, changes)) {
            is Plan.Bad -> return p.outcome
            is Plan.Ok -> p.write
        }
        val changed = try { editor.update(request.eventId, write) } catch (e: CalendarStoreException) { return CalendarOutcome.Unavailable(CalendarModifyLimits.REASON_PROVIDER_ERROR) }
        if (!changed) return CalendarOutcome.Unavailable(CalendarModifyLimits.REASON_UPDATE_REFUSED)
        // Written before the read-back, so a crash between the two still finds the event on a retry.
        ledger.put(request.requestId, request.eventId)
        return verifyUpdate(request.eventId, write)
    }

    fun delete(request: ModifyRequest): CalendarOutcome {
        if (!editor.hasPermission()) return permissionDenied()
        ledger.get(request.requestId)?.let { return verifyDelete(it) }
        when (val g = gate(request)) {
            is Gate.Stop -> return g.outcome
            is Gate.Go -> Unit
        }
        val removed = try { editor.delete(request.eventId) } catch (e: CalendarStoreException) { return CalendarOutcome.Unavailable(CalendarModifyLimits.REASON_PROVIDER_ERROR) }
        if (!removed) return CalendarOutcome.Unavailable(CalendarModifyLimits.REASON_DELETE_REFUSED)
        ledger.put(request.requestId, request.eventId)
        return verifyDelete(request.eventId)
    }

    private sealed interface Gate {
        class Stop(val outcome: CalendarOutcome) : Gate
        class Go(val row: EventRow) : Gate
    }

    /** The shared checks, in the contract's order, after the permission. */
    private fun gate(request: ModifyRequest): Gate {
        val row = try { editor.row(request.eventId) } catch (e: CalendarStoreException) { return Gate.Stop(CalendarOutcome.Unavailable(CalendarModifyLimits.REASON_PROVIDER_ERROR)) }
        if (row == null || row.deleted) return Gate.Stop(EventNotFound)
        if (row.accessLevel < CalendarModifyLimits.ACCESS_EDITOR) return Gate.Stop(ReadOnlyCalendar)
        if (row.recurring) return Gate.Stop(RecurringUnsupported)
        if (cleanPhoneText(row.title) != cleanPhoneText(request.expectedTitle) || !beginMatches(row, request.beginMs)) return Gate.Stop(EventChanged)
        return Gate.Go(row)
    }

    /** All-day rows are UTC midnights, so they are matched by date; timed rows by the instant. */
    private fun beginMatches(row: EventRow, beginMs: Long): Boolean =
        if (row.allDay) utcDate(row.startMs) == utcDate(beginMs) else row.startMs == beginMs

    private fun utcDate(ms: Long): LocalDate = Instant.ofEpochMilli(ms).atOffset(ZoneOffset.UTC).toLocalDate()

    private sealed interface Plan {
        class Ok(val write: EventWrite) : Plan
        class Bad(val outcome: CalendarOutcome.InvalidEvent) : Plan
    }

    /** Only the changed columns. A new start alone keeps the event's length. */
    private fun plan(row: EventRow, c: EventWrite): Plan {
        if (row.allDay && listOfNotNull(c.startMs, c.endMs).any { it % CalendarModifyLimits.DAY_MS != 0L }) {
            return Plan.Bad(CalendarOutcome.InvalidEvent("kind_change", if (c.startMs != null) "new_start_local" else "new_end_local"))
        }
        val start = c.startMs ?: row.startMs
        val end = c.endMs ?: if (c.startMs != null) start + (row.endMs - row.startMs) else row.endMs
        if (end <= start) return Plan.Bad(CalendarOutcome.InvalidEvent("end_not_after_start", "new_end_local"))
        val touchesTime = c.startMs != null || c.endMs != null
        return Plan.Ok(EventWrite(c.title, if (touchesTime) start else null, if (touchesTime) end else null))
    }

    private fun verifyUpdate(eventId: Long, write: EventWrite): CalendarOutcome {
        val row = try { editor.row(eventId) } catch (e: CalendarStoreException) { return UpdateUnverified(CalendarModifyLimits.REASON_READ_FAILED) }
        if (row == null || row.deleted) return UpdateUnverified(CalendarModifyLimits.REASON_EVENT_MISSING)
        val same = (write.title == null || row.title == write.title) && (write.startMs == null || row.startMs == write.startMs) && (write.endMs == null || row.endMs == write.endMs)
        return if (same) Updated(eventId.toString()) else UpdateUnverified(CalendarModifyLimits.REASON_MISMATCH)
    }

    /** A retry of an update that already ran: only what the request states can be checked, since the old length is gone. */
    private fun replayUpdate(eventId: Long, changes: EventWrite): CalendarOutcome = verifyUpdate(eventId, changes)

    private fun verifyDelete(eventId: Long): CalendarOutcome {
        val row = try { editor.row(eventId) } catch (e: CalendarStoreException) { return DeleteUnverified(CalendarModifyLimits.REASON_READ_FAILED) }
        return if (row == null || row.deleted) Deleted(eventId.toString()) else DeleteUnverified(CalendarModifyLimits.REASON_STILL_PRESENT)
    }

    private fun permissionDenied() = CalendarOutcome.PermissionDenied(listOf(CalendarCreator.READ_CALENDAR, CalendarCreator.WRITE_CALENDAR), CalendarCreator.NEXT_STEP_PERMISSION)
}
