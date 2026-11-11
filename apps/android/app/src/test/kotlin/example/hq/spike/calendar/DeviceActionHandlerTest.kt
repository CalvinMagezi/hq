package example.hq.spike.calendar

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

    private val ledger = object : CalendarLedger { override fun get(requestId: String): Long? = null; override fun put(requestId: String, eventId: Long) {} }
    private val none = object : PreferredCalendar { override fun get(): Long? = null }
    private val logs = mutableListOf<String>()
    private fun handler(link: Link, now: Instant = Instant.parse("2026-11-10T00:00:00Z")) = DeviceActionHandler(link, CalendarCreator(Denied(), ledger, none), { now }, { logs += it })

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
}
