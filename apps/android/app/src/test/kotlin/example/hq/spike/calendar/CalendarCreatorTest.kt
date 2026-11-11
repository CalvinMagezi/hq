package example.hq.spike.calendar

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CalendarCreatorTest {
    private val home = WritableCalendar(1, "Home", "me")
    private val work = WritableCalendar(2, "Work", "corp")

    private class FakeStore(var allowed: Boolean = true, var calendars: List<WritableCalendar>, var failInsert: Boolean = false, var corrupt: Boolean = false) : CalendarStore {
        val rows = mutableMapOf<Long, StoredEvent>()
        var inserts = 0
        private var next = 100L
        override fun hasPermission() = allowed
        override fun writableCalendars() = calendars
        override fun insert(calendarId: Long, request: CalendarRequest): Long? {
            if (failInsert) throw CalendarStoreException("no")
            inserts++
            val id = next++
            rows[id] = StoredEvent(calendarId, if (corrupt) "other" else request.title, request.span.startMs, request.span.endMs, request.span.timeZone, request.allDay)
            return id
        }
        override fun read(eventId: Long) = rows[eventId]
    }

    private class MemLedger : CalendarLedger {
        val map = mutableMapOf<String, Long>()
        override fun get(requestId: String) = map[requestId]
        override fun put(requestId: String, eventId: Long) { map[requestId] = eventId }
    }

    private fun request(id: String = "3f2b8a52-6f0e-4d3e-9a55-0d0c3c1f7a11") = (CalendarRequests.parse(
        JSONObject().put("request_id", id).put("title", "Dentist").put("start_local", "2026-11-12T10:00").put("time_zone", "Europe/London").put("duration_minutes", 60).put("duration_defaulted", true),
    ) as ParsedRequest.Ok).request

    private fun creator(store: CalendarStore, ledger: CalendarLedger = MemLedger(), preferred: Long? = 1L) = CalendarCreator(store, ledger, object : PreferredCalendar { override fun get() = preferred })

    @Test fun createsAndReadsBack() {
        val store = FakeStore(calendars = listOf(home, work))
        val out = creator(store).create(request()) as CalendarOutcome.Created
        assertEquals("Home (me)", out.calendarLabel)
        assertEquals("100", out.eventRef)
        assertTrue(out.durationDefaulted)
        assertEquals(1, store.inserts)
    }

    @Test fun missingPermissionNeverTouchesTheProvider() {
        val store = FakeStore(allowed = false, calendars = listOf(home))
        val out = creator(store).create(request())
        assertTrue(out is CalendarOutcome.PermissionDenied)
        assertEquals(0, store.inserts)
    }

    @Test fun noPreferredCalendarAsksEvenWithOneCalendar() {
        val store = FakeStore(calendars = listOf(home))
        val out = creator(store, preferred = null).create(request()) as CalendarOutcome.Clarification
        assertEquals("no_preferred_calendar", out.reason)
        assertEquals(listOf("Home (me)"), out.choices)
        assertEquals(0, store.inserts)
    }

    @Test fun removedOrReadOnlyPreferredCalendarAsks() {
        val store = FakeStore(calendars = listOf(work))
        assertTrue(creator(store, preferred = 1L).create(request()) is CalendarOutcome.Clarification)
        assertNull(CalendarCreator.resolve(listOf(work), 1L))
    }

    @Test fun noWritableCalendarIsUnavailable() {
        assertEquals("no_writable_calendar", (creator(FakeStore(calendars = emptyList())).create(request()) as CalendarOutcome.Unavailable).reason)
    }

    @Test fun sameRequestIdNeverInsertsTwice() {
        val store = FakeStore(calendars = listOf(home))
        val ledger = MemLedger()
        val c = creator(store, ledger)
        val first = c.create(request()) as CalendarOutcome.Created
        val again = c.create(request()) as CalendarOutcome.Created
        assertEquals(first.eventRef, again.eventRef)
        assertEquals(1, store.inserts)
    }

    @Test fun ledgerHitWithDeletedEventIsUnverifiedAndDoesNotReinsert() {
        val store = FakeStore(calendars = listOf(home))
        val ledger = MemLedger()
        val c = creator(store, ledger)
        c.create(request())
        store.rows.clear()
        val out = c.create(request()) as CalendarOutcome.Unverified
        assertEquals("event_missing", out.reason)
        assertEquals(1, store.inserts)
    }

    @Test fun readBackMismatchIsUnverified() {
        val store = FakeStore(calendars = listOf(home), corrupt = true)
        assertEquals("read_back_mismatch", (creator(store).create(request()) as CalendarOutcome.Unverified).reason)
    }

    @Test fun providerFailureIsUnavailableAndLeavesNoLedgerEntry() {
        val ledger = MemLedger()
        val out = creator(FakeStore(calendars = listOf(home), failInsert = true), ledger).create(request())
        assertTrue(out is CalendarOutcome.Unavailable)
        assertTrue(ledger.map.isEmpty())
    }
}
