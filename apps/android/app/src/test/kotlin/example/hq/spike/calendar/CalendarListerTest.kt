package example.hq.spike.calendar

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CalendarListerTest {
    private val day = 24L * 60 * 60 * 1000

    private class FakeReader(var allowed: Boolean = true, val rows: List<CalendarInstance>, var fail: Boolean = false) : CalendarReader {
        val windows = mutableListOf<Pair<Long, Long>>()
        override fun hasPermission() = allowed
        override fun instances(fromMs: Long, toMs: Long): List<CalendarInstance> {
            if (fail) throw CalendarStoreException("no")
            windows += fromMs to toMs
            return rows.filter { it.beginMs < toMs && it.endMs >= fromMs }.shuffled()
        }
    }

    private fun inst(id: Long, begin: Long, end: Long = begin + 1, visible: Boolean = true, allDay: Boolean = false) = CalendarInstance(id, "t$id", begin, end, allDay, "Home", visible)
    private fun req(from: Long = 0, to: Long = 400 * day, limit: Int = 50, zone: String = "UTC") = CalendarListRequest("3f2b8a52-6f0e-4d3e-9a55-0d0c3c1f7a12", from, to, zone, limit)
    private fun listed(r: CalendarOutcome) = r as Listed

    @Test fun capsAtLimitAndPointsAtTheFirstOmittedEvent() {
        val rows = (0 until 120L).map { inst(it, it * day + 5) }
        val out = listed(CalendarLister(FakeReader(rows = rows)).list(req()))
        assertEquals(50, out.events.size)
        assertTrue(out.truncated)
        assertEquals(50 * day + 5, out.nextFromMs)
        assertEquals((0L until 50L).toList(), out.events.map { it.id })
    }

    @Test fun anUnboundedSpanStopsWalkingOnceFull() {
        val reader = FakeReader(rows = (0 until 60L).map { inst(it, it * day) })
        CalendarLister(reader).list(req(to = 100_000 * day))
        assertTrue(reader.windows.size <= 3)
        assertTrue(reader.windows.all { it.second - it.first <= 30 * day })
    }

    @Test fun fewerThanTheLimitIsNotTruncated() {
        val out = listed(CalendarLister(FakeReader(rows = listOf(inst(1, 10), inst(2, 45 * day)))).list(req()))
        assertEquals(2, out.events.size)
        assertFalse(out.truncated)
        assertNull(out.nextFromMs)
        assertTrue(out.toJson().isNull("next_from_ms"))
    }

    @Test fun exactlyTheLimitIsNotTruncated() {
        val out = listed(CalendarLister(FakeReader(rows = (0 until 50L).map { inst(it, it + 1) })).list(req()))
        assertEquals(50, out.events.size)
        assertFalse(out.truncated)
    }

    @Test fun sortsByBeginThenIdAndSkipsRepeatsAcrossWindows() {
        val rows = listOf(inst(9, 5), inst(3, 5), inst(4, 1), inst(5, 29 * day, 31 * day))
        val out = listed(CalendarLister(FakeReader(rows = rows)).list(req()))
        assertEquals(listOf(4L, 3L, 9L, 5L), out.events.map { it.id })
    }

    @Test fun hiddenCalendarsAreSkipped() {
        val out = listed(CalendarLister(FakeReader(rows = listOf(inst(1, 5, visible = false), inst(2, 6)))).list(req()))
        assertEquals(listOf(2L), out.events.map { it.id })
    }

    @Test fun permissionDeniedNamesReadCalendarAndTouchesNothing() {
        val reader = FakeReader(allowed = false, rows = listOf(inst(1, 5)))
        val out = CalendarLister(reader).list(req())
        assertEquals("permission_denied", out.code)
        assertTrue(reader.windows.isEmpty())
        assertTrue(out.toJson().getString("next_step").contains("Calendar"))
    }

    @Test fun providerFailureIsUnavailable() {
        assertEquals("calendar_unavailable", CalendarLister(FakeReader(rows = emptyList(), fail = true)).list(req()).code)
    }

    @Test fun allDayRowsKeepTheirStoredMidnights() {
        val begin = 1_794_000_000_000L - 1_794_000_000_000L % day
        val out = listed(CalendarLister(FakeReader(rows = listOf(inst(1, begin, begin + day, allDay = true)))).list(req(from = begin - day, to = begin + 2 * day)))
        val e = out.events.single()
        assertTrue(e.allDay)
        assertEquals(begin, e.beginMs)
        assertEquals(begin + day, e.endMs)
    }

    @Test fun resultCarriesOnlyTheContractFields() {
        val body = CalendarLister(FakeReader(rows = listOf(inst(1, 5)))).list(req()).toJson()
        assertEquals(setOf("v", "outcome", "events", "truncated", "next_from_ms"), body.keys().asSequence().toSet())
        val event = body.getJSONArray("events").getJSONObject(0)
        assertEquals(setOf("id", "begin_ms", "end_ms", "all_day", "title", "calendar_label"), event.keys().asSequence().toSet())
    }

    @Test fun parseChecksTheRequest() {
        fun p(o: JSONObject) = CalendarListRequests.parse(o)
        val ok = { JSONObject().put("request_id", "3f2b8a52-6f0e-4d3e-9a55-0d0c3c1f7a12").put("from_ms", 1L).put("to_ms", 2L).put("zone", "Europe/London") }
        assertEquals(50, (p(ok()) as ParsedListRequest.Ok).request.limit)
        assertEquals(50, (p(ok().put("limit", 900)) as ParsedListRequest.Ok).request.limit)
        assertTrue(p(ok().put("limit", 0)) is ParsedListRequest.Bad)
        assertTrue(p(ok().put("to_ms", 1L)) is ParsedListRequest.Bad)
        assertTrue(p(ok().put("from_ms", -1L)) is ParsedListRequest.Bad)
        assertTrue(p(ok().put("zone", "Mars/Base")) is ParsedListRequest.Bad)
        assertTrue(p(ok().put("request_id", "nope")) is ParsedListRequest.Bad)
        assertTrue(p(ok().put("location", "x")) is ParsedListRequest.Bad)
        assertTrue(p(JSONObject().put("request_id", "3f2b8a52-6f0e-4d3e-9a55-0d0c3c1f7a12").put("zone", "UTC")) is ParsedListRequest.Bad)
    }

    private val nov11 = 1794355200000L // 2026-11-11T00:00Z

    private fun ids(zone: String, fromMs: Long, toMs: Long, rows: List<CalendarInstance>) =
        listed(CalendarLister(FakeReader(rows = rows)).list(req(fromMs, toMs, zone = zone))).events.map { it.id }

    @Test fun allDayIsJudgedByDateEastOfUtc() {
        val veterans = inst(1, nov11, nov11 + day, allDay = true)
        val nairobi = "Africa/Nairobi"
        val nov12From = nov11 + day - 3 * 3600_000L
        assertEquals(emptyList<Long>(), ids(nairobi, nov12From, nov12From + day, listOf(veterans)))
        val nov11From = nov11 - 3 * 3600_000L
        assertEquals(listOf(1L), ids(nairobi, nov11From, nov11From + day, listOf(veterans)))
    }

    @Test fun allDayIsJudgedByDateWestOfUtc() {
        val veterans = inst(1, nov11, nov11 + day, allDay = true)
        val ny = "America/New_York"
        val nov10From = nov11 - day + 5 * 3600_000L
        assertEquals(emptyList<Long>(), ids(ny, nov10From, nov10From + day, listOf(veterans)))
        val nov11From = nov11 + 5 * 3600_000L
        assertEquals(listOf(1L), ids(ny, nov11From, nov11From + day, listOf(veterans)))
        val nov12From = nov11From + day
        assertEquals(emptyList<Long>(), ids(ny, nov12From, nov12From + day, listOf(veterans)))
    }

    @Test fun aMultiDayAllDayEventOverlappingTheWindowIsIncluded() {
        val trip = inst(2, nov11 - day, nov11 + 3 * day, allDay = true)
        val nairobiNov12 = nov11 + day - 3 * 3600_000L
        assertEquals(listOf(2L), ids("Africa/Nairobi", nairobiNov12, nairobiNov12 + day, listOf(trip)))
        val nyNov12 = nov11 + day + 5 * 3600_000L
        assertEquals(listOf(2L), ids("America/New_York", nyNov12, nyNov12 + day, listOf(trip)))
    }

    @Test fun aTimedEventEndingExactlyAtTheWindowStartIsOut() {
        val from = 10 * day
        val rows = listOf(inst(1, from - 3600_000L, from), inst(2, from - 3600_000L, from + 1), inst(3, from, from), inst(4, from + day, from + day))
        assertEquals(listOf(2L, 3L), ids("UTC", from, from + day, rows))
    }
}
