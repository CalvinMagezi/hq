package example.hq.spike.data

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class OfferTest {
    private fun reply(meta: String?) = JSONObject(
        """{"message_id":"a1","thread_id":"t","role":"assistant","content":"done","created_at":"2026-11-05T00:00:00Z"${if (meta != null) ",\"meta\":$meta" else ""}}""",
    )

    @Test
    fun anOfferUnderAReplyIsRead() {
        val m = StoredMessage.from(reply("""{"offer":{"kind":"pasted_list","line":"I can also turn a list you paste into separate tasks.","reply":"Turn that list into tasks.","send":true}}"""))
        assertEquals(Offer("pasted_list", "I can also turn a list you paste into separate tasks.", "Turn that list into tasks.", true), m.offer)
    }

    @Test
    fun aBrokenOrMissingOfferIsDropped() {
        assertNull(StoredMessage.from(reply("""{"offer":{"kind":"pasted_list","line":"","reply":"x"}}""")).offer)
        assertNull(StoredMessage.from(reply(null)).offer)
    }
}
