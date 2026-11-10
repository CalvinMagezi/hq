package example.hq.spike.calendar

import java.time.Instant
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class CalendarRequestsTest {
    private val id = "3f2b8a52-6f0e-4d3e-9a55-0d0c3c1f7a11"

    private fun payload(vararg extra: Pair<String, Any?>) = JSONObject().put("request_id", id).put("title", "  Dentist ").put("start_local", "2026-11-12T10:00")
        .put("time_zone", "Europe/London").apply { extra.forEach { (k, v) -> put(k, v ?: JSONObject.NULL) } }

    private fun ok(p: JSONObject) = (CalendarRequests.parse(p) as ParsedRequest.Ok).request
    private fun bad(p: JSONObject) = (CalendarRequests.parse(p) as ParsedRequest.Bad).outcome

    @Test fun timedWithDurationTrimsTitleAndComputesEnd() {
        val r = ok(payload("duration_minutes" to 90, "duration_defaulted" to false))
        assertEquals("Dentist", r.title)
        assertEquals(Instant.parse("2026-11-12T10:00:00Z").toEpochMilli(), r.span.startMs)
        assertEquals(Instant.parse("2026-11-12T11:30:00Z").toEpochMilli(), r.span.endMs)
        assertEquals("Europe/London", r.span.timeZone)
        assertEquals("2026-11-12T11:30", r.endLocal)
    }

    @Test fun summerZoneOffsetIsApplied() {
        val r = ok(payload("start_local" to "2026-07-01T10:00", "duration_minutes" to 60))
        assertEquals(Instant.parse("2026-07-01T09:00:00Z").toEpochMilli(), r.span.startMs)
    }

    @Test fun endLocalIsUsedAndDefaultedFlagCarried() {
        val r = ok(payload("end_local" to "2026-11-12T10:45", "duration_defaulted" to true))
        assertEquals(45 * 60_000L, r.span.endMs - r.span.startMs)
        assertTrue(r.durationDefaulted)
    }

    @Test fun endAndDurationTogetherAreInvalid() {
        assertEquals("duration_minutes", bad(payload("end_local" to "2026-11-12T11:00", "duration_minutes" to 30)).field)
    }

    @Test fun endBeforeStartIsInvalid() {
        assertEquals("end_not_after_start", bad(payload("end_local" to "2026-11-12T09:00")).reason)
    }

    @Test fun durationBoundsAreChecked() {
        assertEquals("duration_range", bad(payload("duration_minutes" to 0)).reason)
        assertEquals("duration_range", bad(payload("duration_minutes" to 1441)).reason)
        ok(payload("duration_minutes" to 1440))
    }

    @Test fun titleBoundsAreChecked() {
        assertEquals("title", bad(payload("title" to "   ")).field)
        assertEquals("title", bad(payload("title" to "x".repeat(201))).field)
        ok(payload("title" to "x".repeat(200), "duration_minutes" to 5))
    }

    @Test fun locationAndDescriptionBounds() {
        assertEquals("location", bad(payload("location" to "x".repeat(201))).field)
        assertEquals("description", bad(payload("description" to "x".repeat(2001))).field)
    }

    @Test fun unknownAndUnsupportedFieldsAreRejected() {
        assertEquals("attendees", bad(payload("attendees" to "a")).field)
        assertEquals("recurrence", bad(payload("recurrence" to "daily")).field)
    }

    @Test fun badIdZoneAndStartAreRejected() {
        assertEquals("request_id", bad(payload("request_id" to "not-a-uuid")).field)
        assertEquals("time_zone", bad(payload("time_zone" to "Mars/Base")).field)
        assertEquals("start_local", bad(payload("start_local" to "tomorrow")).field)
    }

    @Test fun nonexistentLocalTimeIsInvalid() {
        // 2026-03-29 01:30 does not exist in London (clocks jump from 01:00 to 02:00).
        assertEquals("nonexistent_local_time", bad(payload("start_local" to "2026-03-29T01:30")).reason)
    }

    @Test fun allDayIsUtcMidnightWithExclusiveEnd() {
        val r = ok(payload("all_day" to true, "start_local" to "2026-11-12", "duration_defaulted" to true))
        assertEquals(Instant.parse("2026-11-12T00:00:00Z").toEpochMilli(), r.span.startMs)
        assertEquals(Instant.parse("2026-11-13T00:00:00Z").toEpochMilli(), r.span.endMs)
        assertEquals("UTC", r.span.timeZone)
    }

    @Test fun allDayUsesUtcEvenInAFarZone() {
        val r = ok(payload("all_day" to true, "start_local" to "2026-11-12", "time_zone" to "Pacific/Auckland"))
        assertEquals(Instant.parse("2026-11-12T00:00:00Z").toEpochMilli(), r.span.startMs)
    }

    @Test fun allDayEndDateIsTheLastDayInclusiveAndDurationIsOneDay() {
        assertEquals(4 * 86_400_000L, ok(payload("all_day" to true, "start_local" to "2026-11-12", "end_local" to "2026-11-15")).span.let { it.endMs - it.startMs })
        assertEquals(86_400_000L, ok(payload("all_day" to true, "start_local" to "2026-11-12", "end_local" to "2026-11-12")).span.let { it.endMs - it.startMs })
        assertEquals(86_400_000L, ok(payload("all_day" to true, "start_local" to "2026-11-12", "duration_minutes" to 1440)).span.let { it.endMs - it.startMs })
        assertEquals("2026-11-15", ok(payload("all_day" to true, "start_local" to "2026-11-12", "end_local" to "2026-11-15")).endLocal)
        assertEquals("end_not_after_start", bad(payload("all_day" to true, "start_local" to "2026-11-12", "end_local" to "2026-11-11")).reason)
    }

    @Test fun outcomesUseTheContractShapes() {
        val created = CalendarOutcome.Created("42", "Dentist", "2026-11-12T10:00", "2026-11-12T11:00", "Europe/London", "Home", true).toJson()
        assertEquals("created", created.getString("outcome"))
        assertEquals("device_calendar", created.getString("saved_to"))
        assertEquals(1, created.getInt("v"))
        val asked = CalendarOutcome.Clarification("no_preferred_calendar", "Which?", listOf("A", "B")).toJson()
        assertEquals(2, asked.getJSONArray("choices").length())
        assertEquals("read_calendar", CalendarOutcome.PermissionDenied(listOf("read_calendar", "write_calendar"), "x").toJson().getJSONArray("missing").getString(0))
    }
}
