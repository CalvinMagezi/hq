package example.hq.spike.calendar

import example.hq.spike.deviceactions.RequestLedger

/** One calendar the owner can save to. [label] is what the owner sees; it is never logged. */
data class WritableCalendar(val id: Long, val displayName: String, val accountLabel: String) {
    val label: String get() = if (accountLabel.isBlank()) displayName else "$displayName ($accountLabel)"
}

/** What the provider holds for a row, read back after the insert. */
data class StoredEvent(val calendarId: Long, val title: String, val startMs: Long, val endMs: Long, val timeZone: String, val allDay: Boolean)

/** The Calendar Provider behind a seam, so the steps run on the JVM against a fake. */
interface CalendarStore {
    fun hasPermission(): Boolean
    fun writableCalendars(): List<WritableCalendar>

    /** The new row's id, or null when the provider refused. May throw [CalendarStoreException]. */
    fun insert(calendarId: Long, request: CalendarRequest): Long?

    /** Null when the row is gone. May throw [CalendarStoreException]. */
    fun read(eventId: Long): StoredEvent?
}

class CalendarStoreException(message: String, cause: Throwable? = null) : Exception(message, cause)

typealias CalendarLedger = RequestLedger

/** The owner's chosen calendar, by id. */
interface PreferredCalendar {
    fun get(): Long?
}

/** The app-side steps of one calendar.create, after the request has parsed. Pure apart from the three seams. */
class CalendarCreator(private val store: CalendarStore, private val ledger: CalendarLedger, private val preferred: PreferredCalendar) {

    fun create(request: CalendarRequest): CalendarOutcome {
        if (!store.hasPermission()) return permissionDenied()
        val earlier = ledger.get(request.requestId)
        if (earlier != null) return replay(request, earlier)
        val calendars = store.writableCalendars()
        val target = resolve(calendars, preferred.get()) ?: return noPreferred(calendars)
        val eventId = try { store.insert(target.id, request) } catch (e: CalendarStoreException) { return CalendarOutcome.Unavailable(REASON_PROVIDER_ERROR) }
            ?: return CalendarOutcome.Unavailable(REASON_INSERT_REFUSED)
        // Written before the read-back, so a crash between the two still finds the row on a retry.
        ledger.put(request.requestId, eventId)
        return verify(request, eventId, target)
    }

    private fun replay(request: CalendarRequest, eventId: Long): CalendarOutcome {
        val stored = try { store.read(eventId) } catch (e: CalendarStoreException) { return CalendarOutcome.Unverified(REASON_READ_FAILED) }
            ?: return CalendarOutcome.Unverified(REASON_EVENT_MISSING)
        val label = store.writableCalendars().firstOrNull { it.id == stored.calendarId }?.label.orEmpty()
        return matchOrUnverified(request, eventId, stored, label)
    }

    private fun verify(request: CalendarRequest, eventId: Long, target: WritableCalendar): CalendarOutcome {
        val stored = try { store.read(eventId) } catch (e: CalendarStoreException) { return CalendarOutcome.Unverified(REASON_READ_FAILED) }
            ?: return CalendarOutcome.Unverified(REASON_EVENT_MISSING)
        return matchOrUnverified(request, eventId, stored, target.label, target.id)
    }

    private fun matchOrUnverified(request: CalendarRequest, eventId: Long, stored: StoredEvent, label: String, expectedCalendar: Long? = null): CalendarOutcome {
        val same = stored.title == request.title && stored.startMs == request.span.startMs && stored.endMs == request.span.endMs &&
            stored.timeZone == request.span.timeZone && stored.allDay == request.allDay && (expectedCalendar == null || stored.calendarId == expectedCalendar)
        if (!same) return CalendarOutcome.Unverified(REASON_MISMATCH)
        return CalendarOutcome.Created(eventId.toString(), request.title, request.startLocal, request.endLocal, request.span.timeZone, label, request.durationDefaulted)
    }

    private fun permissionDenied() = CalendarOutcome.PermissionDenied(listOf(READ_CALENDAR, WRITE_CALENDAR), NEXT_STEP_PERMISSION)

    private fun noPreferred(calendars: List<WritableCalendar>): CalendarOutcome {
        if (calendars.isEmpty()) return CalendarOutcome.Unavailable(REASON_NO_WRITABLE)
        return CalendarOutcome.Clarification(REASON_NO_PREFERRED, ASK_WHICH_CALENDAR, calendars.map { it.label })
    }

    companion object {
        const val READ_CALENDAR = "read_calendar"
        const val WRITE_CALENDAR = "write_calendar"
        const val NEXT_STEP_PERMISSION = "Ask the owner to allow calendar access in HQ's phone settings, under Calendar."
        const val ASK_WHICH_CALENDAR = "Which calendar should HQ save events to? The owner can pick one in HQ settings, under Calendar."
        const val REASON_NO_PREFERRED = "no_preferred_calendar"
        const val REASON_NO_WRITABLE = "no_writable_calendar"
        const val REASON_PROVIDER_ERROR = "provider_error"
        const val REASON_INSERT_REFUSED = "insert_refused"
        const val REASON_EVENT_MISSING = "event_missing"
        const val REASON_READ_FAILED = "read_back_failed"
        const val REASON_MISMATCH = "read_back_mismatch"

        /** The preferred calendar if it is still there and writable; never a guess, even with one calendar on the phone. */
        fun resolve(calendars: List<WritableCalendar>, preferredId: Long?): WritableCalendar? =
            if (preferredId == null) null else calendars.firstOrNull { it.id == preferredId }
    }
}
