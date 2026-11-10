package example.hq.spike.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class DeviceActionEventTest {
    @Test fun deviceActionFrameParses() {
        val e = ChatEvent.parse("""{"type":"device_action","action_id":"abc","thread_id":"t1","tool_call_id":"c1","name":"calendar.create","v":1,"payload":{"title":"x"},"expires_at":"2026-11-10T00:00:20Z"}""") as ChatEvent.DeviceAction
        assertEquals("abc", e.actionId)
        assertEquals("calendar.create", e.name)
        assertEquals(1, e.version)
        assertEquals("x", e.payload.getString("title"))
    }

    @Test fun frameWithoutIdOrPayloadIsDropped() {
        assertNull(ChatEvent.parse("""{"type":"device_action","payload":{}}"""))
        assertNull(ChatEvent.parse("""{"type":"device_action","action_id":"abc"}"""))
    }
}
