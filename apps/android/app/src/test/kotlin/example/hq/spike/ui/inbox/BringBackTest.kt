package example.hq.spike.ui.inbox

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class BringBackTest {
    @Test fun theThreadIsReadFromTheChatLink() {
        assertEquals("t-123", ChatLink.threadOf("/chat?thread=t-123"))
        assertEquals("t-123", ChatLink.threadOf("/chat?x=1&thread=t-123&y=2"))
        assertEquals("a b", ChatLink.threadOf("/chat?thread=a%20b"))
        assertEquals("t1", ChatLink.threadOf("  /chat?thread=t1#top "))
    }

    @Test fun noLinkOrAnotherPageOrAMalformedLinkGivesNoThread() {
        assertNull(ChatLink.threadOf(null))
        assertNull(ChatLink.threadOf(""))
        assertNull(ChatLink.threadOf("/chat"))
        assertNull(ChatLink.threadOf("/chat?thread="))
        assertNull(ChatLink.threadOf("/chat?thread=%20"))
        assertNull(ChatLink.threadOf("/tasks?thread=t1"))
        assertNull(ChatLink.threadOf("/chat?other=t1"))
        assertNull(ChatLink.threadOf("https://example.com/x?thread=t1"))
    }

    @Test fun aSecondTapWhileTheFirstIsOnItsWaySendsNothing() {
        val g = BringBackGuard()
        assertTrue(g.begin("c1"))
        assertFalse(g.begin("c1"))
        assertTrue(g.begin("c2"))
        g.end("c1")
        assertTrue(g.begin("c1"))
    }
}
