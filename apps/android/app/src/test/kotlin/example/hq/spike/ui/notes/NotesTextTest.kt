package example.hq.spike.ui.notes

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import java.time.Instant
import org.junit.Assert.assertEquals
import org.junit.Test

class NotesTextTest {
    private val now = Instant.parse("2026-10-21T12:00:00Z")

    @Test fun listAgesAreShortAndReadHqTimes() {
        assertEquals("now", NoteTime.short("2026-10-21T11:59:30.5+00:00", now))
        assertEquals("5m", NoteTime.short("2026-10-21T11:55:00+00:00", now))
        assertEquals("3h", NoteTime.short("2026-10-21T09:00:00+00:00", now))
        assertEquals("1d", NoteTime.short("2026-10-20T11:00:00+00:00", now))
        assertEquals("", NoteTime.short("not a time", now))
        assertEquals("edited 1d ago", NoteTime.edited("2026-10-20T11:00:00+00:00", now))
        assertEquals("edited just now", NoteTime.edited("2026-10-21T11:59:59+00:00", now))
    }

    private fun v(text: String, from: Int = text.length, to: Int = from) = TextFieldValue(text, TextRange(from, to))

    @Test fun headingCyclesThroughTheLevels() {
        var t = MarkdownEditing.heading(v("Plan"))
        assertEquals("## Plan", t.text)
        t = MarkdownEditing.heading(t)
        assertEquals("### Plan", t.text)
        assertEquals("Plan", MarkdownEditing.heading(t).text)
    }

    @Test fun headingOnlyTouchesTheCurrentLine() {
        val t = MarkdownEditing.heading(v("one\ntwo\nthree", from = 6))
        assertEquals("one\n## two\nthree", t.text)
    }

    @Test fun boldWrapsTheSelectionAndUnwraps() {
        val wrapped = MarkdownEditing.wrap(v("a battery b", 2, 9), "**")
        assertEquals("a **battery** b", wrapped.text)
        assertEquals(TextRange(4, 11), wrapped.selection)
        assertEquals("a battery b", MarkdownEditing.wrap(wrapped, "**").text)
    }

    @Test fun boldWithNoSelectionPlacesTheCursorBetweenTheMarks() {
        val t = MarkdownEditing.wrap(v("ab", 1), "*")
        assertEquals("a**b", t.text)
        assertEquals(TextRange(2), t.selection)
    }

    @Test fun listPrefixesEveryTouchedLineAndTogglesOff() {
        val on = MarkdownEditing.prefixLines(v("one\ntwo\nthree", 1, 6), "- ")
        assertEquals("- one\n- two\nthree", on.text)
        assertEquals("one\ntwo\nthree", MarkdownEditing.prefixLines(v(on.text, 1, 9), "- ").text)
    }

    @Test fun linkWrapsTheSelectionInDoubleBrackets() {
        val t = MarkdownEditing.link(v("see the guide", 4, 13))
        assertEquals("see [[the guide]]", t.text)
        assertEquals(17, t.selection.min)
    }

    @Test fun linkWithNothingSelectedLeavesTheCursorBetweenTheBrackets() {
        val t = MarkdownEditing.link(v("see ", 4, 4))
        assertEquals("see [[]]", t.text)
        assertEquals(6, t.selection.min)
    }

    @Test fun theAskPillSaysWhenItTakesOnlyTheFirstTen() {
        assertEquals("Ask HQ about this note", askWords(1))
        assertEquals("Ask HQ about these 7 notes", askWords(7))
        assertEquals("Ask HQ about these 10 notes", askWords(10))
        assertEquals("Ask HQ about the first 10 notes", askWords(13))
    }

    @Test fun makeATaskPointsAtTheNoteAndRepeatsAsTheSameTask() {
        val b = noteTaskBody("Notebooks/Projects/Mobile-First-Roadmap.md", "Mobile-First-Roadmap")
        assertEquals("Mobile-First-Roadmap", b.getString("title"))
        assertEquals("Notebooks/Projects/Mobile-First-Roadmap.md", b.getString("description"))
        assertEquals("note:Notebooks/Projects/Mobile-First-Roadmap.md", b.getString("external_id"))
        assertEquals("Inbox", b.getString("initiative"))
        assertEquals("Mobile-First-Roadmap", noteTaskBody("Notebooks/Projects/Mobile-First-Roadmap.md", "").getString("title"))
    }
}
