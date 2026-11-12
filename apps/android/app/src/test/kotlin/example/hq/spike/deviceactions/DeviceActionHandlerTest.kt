package example.hq.spike.deviceactions

import example.hq.spike.calendar.CalendarCreator
import example.hq.spike.calendar.CalendarEditor
import example.hq.spike.calendar.CalendarInstance
import example.hq.spike.calendar.CalendarLister
import example.hq.spike.calendar.CalendarModifier
import example.hq.spike.calendar.CalendarModule
import example.hq.spike.calendar.CalendarReader
import example.hq.spike.calendar.CalendarRequest
import example.hq.spike.calendar.CalendarStore
import example.hq.spike.calendar.EventRow
import example.hq.spike.calendar.EventWrite
import example.hq.spike.calendar.PreferredCalendar
import example.hq.spike.calendar.StoredEvent
import example.hq.spike.calendar.WritableCalendar

import example.hq.spike.data.ChatEvent
import example.hq.spike.data.HqUnavailable
import java.time.Instant
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class DeviceActionHandlerTest {
    private class Link : DeviceActionLink {
        val acks = mutableListOf<String>()
        val results = mutableListOf<Pair<String, JSONObject>>()
        var failResults = 0
        var open: List<ChatEvent.DeviceAction> = emptyList()
        override suspend fun ack(actionId: String) { acks += actionId }
        override suspend fun result(actionId: String, result: JSONObject) {
            if (failResults > 0) { failResults--; throw HqUnavailable("down") }
            results += actionId to result
        }
        override suspend fun pending() = open
    }

    private class Denied : CalendarStore {
        override fun hasPermission() = false
        override fun writableCalendars() = emptyList<WritableCalendar>()
        override fun insert(calendarId: Long, request: CalendarRequest): Long? = error("must not insert")
        override fun read(eventId: Long): StoredEvent? = null
    }

    private val ledger = object : RequestLedger { override fun get(requestId: String): Long? = null; override fun put(requestId: String, eventId: Long) {} }
    private val none = object : PreferredCalendar { override fun get(): Long? = null }
    private class FakeReader(var allowed: Boolean = true, val rows: List<CalendarInstance> = emptyList()) : CalendarReader {
        override fun hasPermission() = allowed
        override fun instances(fromMs: Long, toMs: Long) = rows.filter { it.beginMs < toMs && it.endMs > fromMs }
    }
    private var reader = FakeReader()
    private class FakeEditor : CalendarEditor {
        var rows = mutableMapOf<Long, EventRow>()
        override fun hasPermission() = true
        override fun row(eventId: Long) = rows[eventId]
        override fun update(eventId: Long, write: EventWrite): Boolean {
            val r = rows[eventId] ?: return false
            rows[eventId] = r.copy(title = write.title ?: r.title)
            return true
        }
        override fun delete(eventId: Long) = rows.remove(eventId) != null
    }
    private val editor = FakeEditor()
    private val logs = mutableListOf<String>()
    private fun handler(link: Link, now: Instant = Instant.parse("2026-11-10T00:00:00Z")) = DeviceActionHandler(link, DeviceActionRegistry(listOf(CalendarModule(CalendarCreator(Denied(), ledger, none), CalendarLister(reader), CalendarModifier(editor, ledger)))), { now }, { logs += it })

    private fun action(id: String = "a1", expires: String? = "2026-11-10T01:00:00Z", name: String = "calendar.create") = ChatEvent.DeviceAction(
        id, "t1", name, 1,
        JSONObject().put("request_id", "3f2b8a52-6f0e-4d3e-9a55-0d0c3c1f7a11").put("title", "Secret title").put("start_local", "2026-11-12T10:00").put("time_zone", "Europe/London").put("duration_minutes", 60),
        expires,
    )

    @Test fun acksThenPostsTheOutcome() = runBlocking {
        val link = Link()
        handler(link).handle(action())
        assertEquals(listOf("a1"), link.acks)
        assertEquals("permission_denied", link.results.single().second.getString("outcome"))
    }

    @Test fun invalidPayloadIsAnInvalidEventResult() = runBlocking {
        val link = Link()
        val bad = action().let { it.copy(payload = JSONObject(it.payload.toString()).put("attendees", "x")) }
        handler(link).handle(bad)
        assertEquals("invalid_event", link.results.single().second.getString("outcome"))
    }

    @Test fun expiredOrForeignActionsAreIgnored() = runBlocking {
        val link = Link()
        val h = handler(link)
        h.handle(action(expires = "2026-11-09T00:00:00Z"))
        h.handle(action(id = "a2", name = "other.thing"))
        assertTrue(link.acks.isEmpty() && link.results.isEmpty())
    }

    @Test fun aRepeatedFrameIsHandledOnce() = runBlocking {
        val link = Link()
        val h = handler(link)
        h.handle(action()); h.handle(action())
        assertEquals(1, link.results.size)
    }

    @Test fun aResultHqNeverTookIsPostedAgainFromThePendingList() = runBlocking {
        val link = Link().apply { failResults = 1; open = listOf(action()) }
        val h = handler(link)
        h.handle(action())
        assertTrue(link.results.isEmpty())
        h.settlePending()
        assertEquals(1, link.results.size)
        assertEquals(1, link.acks.size)
    }

    @Test fun logsCarryNoTitle() = runBlocking {
        handler(Link()).handle(action())
        assertTrue(logs.isNotEmpty() && logs.none { it.contains("Secret title") })
    }

    private fun listAction(id: String = "l1") = ChatEvent.DeviceAction(
        id, "t1", "calendar.list", 1,
        JSONObject().put("request_id", "3f2b8a52-6f0e-4d3e-9a55-0d0c3c1f7a12").put("from_ms", 0L).put("to_ms", 1000L).put("zone", "Europe/London").put("limit", 50),
        "2026-11-10T01:00:00Z",
    )

    @Test fun aListActionIsAckedAndAnswered() = runBlocking {
        reader = FakeReader(rows = listOf(CalendarInstance(7, "Secret title", 100, 200, false, "Home", true)))
        val link = Link()
        handler(link).handle(listAction())
        assertEquals(listOf("l1"), link.acks)
        val body = link.results.single().second
        assertEquals("listed", body.getString("outcome"))
        assertEquals(1, body.getInt("v"))
        assertEquals(7L, body.getJSONArray("events").getJSONObject(0).getLong("id"))
        assertTrue(logs.none { it.contains("Secret title") })
    }

    @Test fun aListWithoutPermissionIsPermissionDenied() = runBlocking {
        reader = FakeReader(allowed = false)
        val link = Link()
        handler(link).handle(listAction())
        assertEquals("permission_denied", link.results.single().second.getString("outcome"))
    }

    @Test fun aBadListPayloadIsInvalidEvent() = runBlocking {
        val link = Link()
        val bad = listAction().let { it.copy(payload = JSONObject(it.payload.toString()).put("location", "x")) }
        handler(link).handle(bad)
        assertEquals("invalid_event", link.results.single().second.getString("outcome"))
    }

    private fun modifyAction(name: String, id: String, requestId: String, changes: JSONObject?) = ChatEvent.DeviceAction(
        id, "t1", name, 1,
        JSONObject().put("request_id", requestId).put("event_id", 7).put("begin_ms", 1000L).put("expected_title", "Secret title").put("zone", "Europe/London")
            .also { if (changes != null) it.put("changes", changes) },
        "2026-11-10T01:00:00Z",
    )

    private fun seedEvent() { editor.rows[7] = EventRow(1, "Secret title", 1000L, 2000L, false, "", "", "", null, false, 700) }

    @Test fun anUpdateActionIsAckedAnsweredAndNotLogged() = runBlocking {
        seedEvent()
        val link = Link()
        handler(link).handle(modifyAction("calendar.update", "u1", "3f2b8a52-6f0e-4d3e-9a55-0d0c3c1f7a31", JSONObject().put("title", "Renamed secret")))
        assertEquals(listOf("u1"), link.acks)
        assertEquals("updated", link.results.single().second.getString("outcome"))
        assertEquals("Renamed secret", editor.rows[7]!!.title)
        assertTrue(logs.none { it.contains("secret", ignoreCase = true) })
    }

    @Test fun aDeleteActionIsAckedAndAnswered() = runBlocking {
        seedEvent()
        val link = Link()
        handler(link).handle(modifyAction("calendar.delete", "d1", "3f2b8a52-6f0e-4d3e-9a55-0d0c3c1f7a32", null))
        assertEquals("deleted", link.results.single().second.getString("outcome"))
        assertTrue(editor.rows.isEmpty())
    }

    @Test fun aBadModifyPayloadIsInvalidEvent() = runBlocking {
        val link = Link()
        handler(link).handle(modifyAction("calendar.delete", "d2", "3f2b8a52-6f0e-4d3e-9a55-0d0c3c1f7a33", JSONObject().put("title", "x")))
        assertEquals("invalid_event", link.results.single().second.getString("outcome"))
    }
}
