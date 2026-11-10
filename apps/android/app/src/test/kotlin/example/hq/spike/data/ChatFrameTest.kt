package example.hq.spike.data

import java.time.Instant
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class ChatFrameTest {
    @Test fun frameCarriesThePhoneZoneAndClock() {
        val f = ChatConnection.chatFrame("hi", "t1", "c1", null, emptyList(), null, zone = "Europe/London", nowUtc = Instant.parse("2026-11-10T12:00:00Z"))
        assertEquals("chat", f.getString("type"))
        assertEquals("Europe/London", f.getJSONObject("device").getString("tz"))
        assertEquals("2026-11-10T12:00:00Z", f.getJSONObject("device").getString("now_utc"))
        assertEquals("hi", f.getString("content"))
    }

    @Test fun optionalFieldsStayOptional() {
        val f = ChatConnection.chatFrame("hi", null, "c1", null, emptyList(), null)
        assertFalse(f.has("replace_from") || f.has("source"))
        assertEquals(true, f.isNull("thread_id"))
        assertEquals(true, f.has("device"))
    }
}
