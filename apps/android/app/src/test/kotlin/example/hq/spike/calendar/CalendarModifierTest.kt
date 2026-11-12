package example.hq.spike.calendar

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class CalendarModifierTest {
    private val hour = 3_600_000L
    private val day = CalendarModifyLimits.DAY_MS
    private val start = 1_800_000_000_000L - 1_800_000_000_000L % day + 10 * hour
    private val rid = "3f2b8a52-6f0e-4d3e-9a55-0d0c3c1f7a21"

    private fun row(title: String = "Dentist", s: Long = start, e: Long = start + hour, allDay: Boolean = false, access: Int = 700) =
        EventRow(1, title, s, e, allDay, "", "", "", null, false, access)

    private class FakeEditor(var allowed: Boolean = true) : CalendarEditor {
        val rows = mutableMapOf<Long, EventRow>()
        var writes = 0
        var deletes = 0
        var failRead = false
        var refuse = false
        var lie = false
        override fun hasPermission() = allowed
        override fun row(eventId: Long): EventRow? { if (failRead) throw CalendarStoreException("x"); return rows[eventId] }
        override fun update(eventId: Long, write: EventWrite): Boolean {
            if (refuse) return false
            writes++
            if (lie) return true
            val r = rows[eventId] ?: return false
            rows[eventId] = r.copy(title = write.title ?: r.title, startMs = write.startMs ?: r.startMs, endMs = write.endMs ?: r.endMs)
            return true
        }
        override fun delete(eventId: Long): Boolean {
            if (refuse) return false
            deletes++
            if (!lie) rows.remove(eventId)
            return true
        }
    }

    private class MemLedger : CalendarLedger {
        val map = mutableMapOf<String, Long>()
        override fun get(requestId: String) = map[requestId]
        override fun put(requestId: String, eventId: Long) { map[requestId] = eventId }
    }

    private val editor = FakeEditor().apply { rows[5] = row() }
    private val ledger = MemLedger()
    private val modifier get() = CalendarModifier(editor, ledger)

    private fun req(changes: EventWrite? = EventWrite("Dentist 2", null, null), title: String = "Dentist", begin: Long = start, id: Long = 5, requestId: String = rid) =
        ModifyRequest(requestId, id, begin, title, "Europe/London", changes)

    @Test fun updatesTitleOnly() {
        val out = modifier.update(req())
        assertEquals(Updated("5"), out)
        assertEquals("Dentist 2", editor.rows[5]!!.title)
        assertEquals(start, editor.rows[5]!!.startMs)
    }

    @Test fun newStartAloneKeepsTheDuration() {
        editor.rows[5] = row(e = start + 2 * hour)
        val out = modifier.update(req(EventWrite(null, start + 3 * hour, null)))
        assertEquals(Updated("5"), out)
        assertEquals(start + 3 * hour, editor.rows[5]!!.startMs)
        assertEquals(start + 5 * hour, editor.rows[5]!!.endMs)
    }

    @Test fun endBeforeStartIsInvalid() {
        val out = modifier.update(req(EventWrite(null, null, start - hour)))
        assertEquals(CalendarOutcome.InvalidEvent("end_not_after_start", "new_end_local"), out)
        assertEquals(0, editor.writes)
    }

    @Test fun allDayEndIsExclusiveMidnightAndInclusiveWireWorks() {
        val d = start - start % day
        editor.rows[5] = row(s = d, e = d + day, allDay = true)
        val out = modifier.update(req(EventWrite(null, null, d + 3 * day), begin = d))
        assertEquals(Updated("5"), out)
        assertEquals(d + 3 * day, editor.rows[5]!!.endMs)
    }

    @Test fun allDayKindChangeIsRejected() {
        val d = start - start % day
        editor.rows[5] = row(s = d, e = d + day, allDay = true)
        val out = modifier.update(req(EventWrite(null, d + 3 * hour, null), begin = d))
        assertEquals("kind_change", (out as CalendarOutcome.InvalidEvent).reason)
        assertEquals(0, editor.writes)
    }

    @Test fun allDayBeginMatchesByUtcDate() {
        val d = start - start % day
        editor.rows[5] = row(s = d, e = d + day, allDay = true)
        assertEquals(Updated("5"), modifier.update(req(begin = d + 5 * hour)))
    }

    @Test fun anUpdateWithTheAllDayFlagParsesAndAKindMismatchIsRejected() {
        val payload = JSONObject()
            .put("request_id", rid).put("event_id", 5).put("begin_ms", start).put("expected_title", "Dentist").put("zone", "Europe/London")
            .put("all_day", true).put("changes", JSONObject().put("title", "Dentist 2"))
        val parsed = CalendarModifyRequests.parseUpdate(payload) as ParsedModify.Ok
        assertEquals(true, parsed.request.allDay)
        assertEquals(CalendarOutcome.InvalidEvent("kind_change", "start_local"), modifier.update(parsed.request))
        assertEquals(0, editor.writes)
    }

    @Test fun aDeleteStillRejectsTheAllDayField() {
        val payload = JSONObject()
            .put("request_id", rid).put("event_id", 5).put("begin_ms", start).put("expected_title", "Dentist").put("zone", "Europe/London")
            .put("all_day", false)
        assertTrue(CalendarModifyRequests.parseDelete(payload) is ParsedModify.Bad)
    }

    @Test fun deletes() {
        assertEquals(Deleted("5"), modifier.delete(req(changes = null)))
        assertEquals(1, editor.deletes)
    }

    @Test fun missingPermissionNeverTouchesTheProvider() {
        editor.allowed = false
        val u = modifier.update(req())
        val d = modifier.delete(req(changes = null))
        assertTrue(u is CalendarOutcome.PermissionDenied && d is CalendarOutcome.PermissionDenied)
        assertEquals(0, editor.writes + editor.deletes)
    }

    @Test fun missingOrDeletedRowIsNotFound() {
        assertEquals(EventNotFound, modifier.delete(req(changes = null, id = 9)))
        editor.rows[5] = row().copy(deleted = true)
        assertEquals(EventNotFound, modifier.update(req()))
    }

    @Test fun readOnlyCalendarIsRefused() {
        editor.rows[5] = row(access = 500)
        assertEquals(ReadOnlyCalendar, modifier.update(req()))
        assertEquals(ReadOnlyCalendar, modifier.delete(req(changes = null)))
        assertEquals(0, editor.writes + editor.deletes)
    }

    @Test fun recurringInAnyFormIsRefused() {
        for (r in listOf(row().copy(rrule = "FREQ=DAILY"), row().copy(rdate = "20261112"), row().copy(originalId = "abc"), row().copy(originalInstanceTime = 5L))) {
            editor.rows[5] = r
            assertEquals(RecurringUnsupported, modifier.update(req()))
            assertEquals(RecurringUnsupported, modifier.delete(req(changes = null)))
        }
        assertEquals(0, editor.writes + editor.deletes)
    }

    @Test fun titleOrStartMismatchIsEventChanged() {
        assertEquals(EventChanged, modifier.update(req(title = "Other")))
        assertEquals(EventChanged, modifier.update(req(begin = start + hour)))
        assertEquals(EventChanged, modifier.delete(req(changes = null, title = "Other")))
        assertEquals(0, editor.writes + editor.deletes)
    }

    @Test fun titlesAreComparedAfterTheCleanRule() {
        editor.rows[5] = row(title = "Den\ntist \u0007 visit")
        assertEquals(Updated("5"), modifier.update(req(title = "Den tist visit")))
    }

    @Test fun readBackMismatchIsUnverified() {
        editor.lie = true
        assertEquals(UpdateUnverified("read_back_mismatch"), modifier.update(req()))
        assertEquals(DeleteUnverified("still_present"), modifier.delete(req(changes = null, requestId = "3f2b8a52-6f0e-4d3e-9a55-0d0c3c1f7a22")))
    }

    @Test fun unreadableReadBackIsUnverifiedNotSuccess() {
        val flaky = object : CalendarEditor by editor {
            var reads = 0
            override fun row(eventId: Long): EventRow? { if (++reads > 1) throw CalendarStoreException("x"); return editor.row(eventId) }
        }
        assertEquals(UpdateUnverified("read_back_failed"), CalendarModifier(flaky, ledger).update(req()))
    }

    @Test fun providerFailuresAreUnavailable() {
        editor.failRead = true
        assertEquals(CalendarOutcome.Unavailable("provider_error"), modifier.update(req()))
        editor.failRead = false
        editor.refuse = true
        assertEquals(CalendarOutcome.Unavailable("update_refused"), modifier.update(req()))
        assertEquals(CalendarOutcome.Unavailable("delete_refused"), modifier.delete(req(changes = null)))
    }

    @Test fun aRetriedUpdateReturnsTheFirstResultWithoutWritingAgain() {
        assertEquals(Updated("5"), modifier.update(req()))
        assertEquals(Updated("5"), modifier.update(req()))
        assertEquals(1, editor.writes)
    }

    @Test fun aRetriedDeleteReturnsTheFirstResult() {
        assertEquals(Deleted("5"), modifier.delete(req(changes = null)))
        assertEquals(Deleted("5"), modifier.delete(req(changes = null)))
        assertEquals(1, editor.deletes)
    }

    @Test fun outcomesCarryTheWireShape() {
        val body = Updated("5").toJson()
        assertEquals(1, body.getInt("v"))
        assertEquals("updated", body.getString("outcome"))
        assertEquals("delete_unverified", DeleteUnverified("x").toJson().getString("outcome"))
        assertEquals("recurring_unsupported", RecurringUnsupported.toJson().getString("outcome"))
    }

    private fun payload(changes: JSONObject? = JSONObject().put("title", "New")) = JSONObject()
        .put("request_id", rid).put("event_id", 5).put("begin_ms", start).put("expected_title", "Dentist").put("zone", "Europe/London")
        .also { if (changes != null) it.put("changes", changes) }

    @Test fun parsesAnUpdateAndADelete() {
        val u = (CalendarModifyRequests.parseUpdate(payload()) as ParsedModify.Ok).request
        assertEquals("New", u.changes!!.title)
        val d = (CalendarModifyRequests.parseDelete(payload(null)) as ParsedModify.Ok).request
        assertEquals(null, d.changes)
        assertEquals(5L, d.eventId)
    }

    private fun reason(p: ParsedModify) = (p as ParsedModify.Bad).outcome.reason

    @Test fun parsingRejectsBadInput() {
        assertEquals("unknown_field", reason(CalendarModifyRequests.parseDelete(payload())))
        assertEquals("unknown_field", reason(CalendarModifyRequests.parseUpdate(payload().put("location", "x"))))
        assertEquals("invalid_request_id", reason(CalendarModifyRequests.parseUpdate(payload().put("request_id", "nope"))))
        assertEquals("invalid_event_id", reason(CalendarModifyRequests.parseUpdate(payload().put("event_id", 0))))
        assertEquals("invalid_time_zone", reason(CalendarModifyRequests.parseUpdate(payload().put("zone", "Mars/Base"))))
        assertEquals("changes_missing", reason(CalendarModifyRequests.parseUpdate(payload(null))))
        assertEquals("no_changes", reason(CalendarModifyRequests.parseUpdate(payload(JSONObject()))))
        assertEquals("unknown_field", reason(CalendarModifyRequests.parseUpdate(payload(JSONObject().put("location", "x")))))
        assertEquals("title_length", reason(CalendarModifyRequests.parseUpdate(payload(JSONObject().put("title", "  ")))))
        assertEquals("invalid_start_ms", reason(CalendarModifyRequests.parseUpdate(payload(JSONObject().put("start_ms", -1)))))
    }
}
