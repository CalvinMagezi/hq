package example.hq.spike.data

import org.junit.Test
import org.junit.Assert.assertEquals

class NoteSearchQueryTest {
    @Test fun `a plain search asks for the words and the limit only`() {
        assertEquals("q=road+map&limit=30", noteSearchQuery("road map", 30, null, false, null))
    }

    @Test fun `written by HQ and a tag are sent as HQ expects them`() {
        assertEquals("q=plan&limit=10&folder=Projects&written_by=hq&tag=roadmap", noteSearchQuery("plan", 10, "Projects", true, "#roadmap"))
    }

    @Test fun `a blank tag is not sent`() {
        assertEquals("q=plan&limit=10", noteSearchQuery("plan", 10, null, false, " "))
    }
}
