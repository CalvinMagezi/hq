package example.hq.spike.ui.chat

import java.time.Instant
import java.time.ZoneOffset
import org.junit.Assert.assertEquals
import org.junit.Test

class ChatTimeAndContextTest {
    private val now = Instant.parse("2026-10-21T12:00:00Z")

    @Test fun sinceUsesTheWordsACardNeeds() {
        fun since(at: String) = ChatTime.since(at, now, ZoneOffset.UTC)
        assertEquals("just now", since("2026-10-21T11:59:40Z"))
        assertEquals("3m ago", since("2026-10-21T11:57:00Z"))
        assertEquals("2h ago", since("2026-10-21T10:00:00Z"))
        assertEquals("yesterday", since("2026-10-20T09:00:00Z"))
        assertEquals("4d ago", since("2026-10-17T09:00:00Z"))
        assertEquals("", since("not a time"))
    }

    @Test fun attachedNotesTravelInAHiddenLeadingBlock() {
        assertEquals("hello", withContext("hello", emptyList()))
        val sent = withContext("hello", listOf("Notebooks/A.md", "Notebooks/B.md"))
        assert(sent.startsWith("<hq-context>")) { sent }
        assert(sent.endsWith("</hq-context>\n\nhello")) { sent }
        assert("Notebooks/A.md, Notebooks/B.md" in sent) { sent }
    }
}
