package example.hq.spike.deviceactions

import example.hq.spike.data.ChatEvent
import example.hq.spike.data.HqUnavailable
import java.time.Instant
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class DeviceActionRegistryTest {
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

    private class Done(val ref: Long) : DeviceOutcome {
        override val code get() = "done"
        override fun toJson(): JSONObject = JSONObject().put("v", 1).put("outcome", code).put("ref", ref)
    }

    private class MemoryLedger : RequestLedger {
        val map = mutableMapOf<String, Long>()
        override fun get(requestId: String) = map[requestId]
        override fun put(requestId: String, eventId: Long) { map[requestId] = eventId }
    }

    /** Acts once per request_id: a repeat reads the ledger instead of making a second thing. */
    private class FakeModule(override val actions: Set<String>, val ledger: RequestLedger = MemoryLedger(), override val wireVersion: Int = 1) : DeviceActionModule {
        var made = 0
        override fun run(action: ChatEvent.DeviceAction): DeviceOutcome {
            val requestId = action.payload.getString("request_id")
            ledger.get(requestId)?.let { return Done(it) }
            made++
            ledger.put(requestId, made.toLong())
            return Done(made.toLong())
        }
    }

    private val now = Instant.parse("2026-11-10T00:00:00Z")
    private fun handler(link: Link, vararg modules: DeviceActionModule) =
        DeviceActionHandler(link, DeviceActionRegistry(modules.toList()), { now }, { })

    private fun action(name: String, id: String = "a1", requestId: String = "r1", version: Int = 1) = ChatEvent.DeviceAction(
        id, "t1", name, version, JSONObject().put("request_id", requestId), "2026-11-10T01:00:00Z",
    )

    @Test fun anUnknownActionIsIgnored() = runBlocking {
        val link = Link()
        handler(link, FakeModule(setOf("fake.make"))).handle(action("nothing.here"))
        assertTrue(link.acks.isEmpty() && link.results.isEmpty())
    }

    @Test fun aWrongWireVersionIsIgnored() = runBlocking {
        val link = Link()
        handler(link, FakeModule(setOf("fake.make"))).handle(action("fake.make", version = 2))
        assertTrue(link.acks.isEmpty() && link.results.isEmpty())
    }

    @Test fun aModuleNextToAnotherHandlesItsOwnActionWithAckAndResult() = runBlocking {
        val link = Link()
        val fake = FakeModule(setOf("fake.make"))
        val other = FakeModule(setOf("other.make"))
        handler(link, other, fake).handle(action("fake.make"))
        assertEquals(listOf("a1"), link.acks)
        assertEquals("done", link.results.single().second.getString("outcome"))
        assertEquals(1, fake.made)
        assertEquals(0, other.made)
    }

    @Test fun aResultHqNeverTookIsPostedAgainFromThePendingList() = runBlocking {
        val fake = FakeModule(setOf("fake.make"))
        val link = Link().apply { failResults = 1; open = listOf(action("fake.make")) }
        val h = handler(link, fake)
        h.handle(action("fake.make"))
        assertTrue(link.results.isEmpty())
        h.settlePending()
        assertEquals(1, link.results.size)
        assertEquals(1, link.acks.size)
        assertEquals(1, fake.made)
    }

    @Test fun aRetryWithTheSameRequestIdDoesNotActTwice() = runBlocking {
        val link = Link()
        val fake = FakeModule(setOf("fake.make"))
        val h = handler(link, fake)
        h.handle(action("fake.make", id = "a1", requestId = "same"))
        h.handle(action("fake.make", id = "a2", requestId = "same"))
        assertEquals(1, fake.made)
        assertEquals(2, link.results.size)
        assertEquals(link.results[0].second.getLong("ref"), link.results[1].second.getLong("ref"))
    }

    @Test fun twoModulesCannotClaimOneActionName() {
        try {
            DeviceActionRegistry(listOf(FakeModule(setOf("fake.make")), FakeModule(setOf("x.y", "fake.make"))))
            fail("expected a duplicate claim to fail")
        } catch (e: IllegalArgumentException) {
            assertTrue(e.message!!.contains("fake.make"))
        }
    }

    @Test fun lookupFindsTheClaimingModule() {
        val fake = FakeModule(setOf("fake.make"))
        val registry = DeviceActionRegistry(listOf(fake))
        assertSame(fake, registry.moduleFor("fake.make"))
        assertNull(registry.moduleFor("fake.other"))
    }
}
