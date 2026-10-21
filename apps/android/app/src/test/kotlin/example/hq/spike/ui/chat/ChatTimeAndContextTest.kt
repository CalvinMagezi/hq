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
        val sent = withContext("hello", listOf(ContextRef(ContextRef.Kind.Note, "Notebooks/A.md", "A"), ContextRef(ContextRef.Kind.Task, "HQ-12", "Check")))
        assert(sent.startsWith("<hq-context>")) { sent }
        assert(sent.endsWith("</hq-context>\n\nhello")) { sent }
        assert("note Notebooks/A.md; task HQ-12 (Check)" in sent) { sent }
    }

    @Test fun theClockShowsTheLocalTimeOfDay() {
        assertEquals("14:28", ChatTime.clock("2026-10-21T12:28:00Z", ZoneOffset.ofHours(2)))
        assertEquals("", ChatTime.clock(""))
    }

    @Test fun slashCommandsAreOnlyTheTwoTheWebHas() {
        assertEquals(SlashCommands.Command("/clear", null), SlashCommands.parse(" /clear "))
        assertEquals(SlashCommands.Command("/topic", "Soak week"), SlashCommands.parse("/Topic Soak week"))
        assertEquals(null, SlashCommands.parse("/shrug"))
        assertEquals(null, SlashCommands.parse("hello /clear"))
    }

    @Test fun thePaletteOpensOnASlashAndClosesOnceAnArgumentIsTyped() {
        assertEquals(listOf("/clear", "/topic"), SlashCommands.matching("/").map { it.first })
        assertEquals(listOf("/topic"), SlashCommands.matching("/to").map { it.first })
        assertEquals(emptyList<Any>(), SlashCommands.matching("/topic Soak"))
        assertEquals(emptyList<Any>(), SlashCommands.matching("hello"))
    }
}
