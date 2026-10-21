package example.hq.spike.ui.chat.blocks

import androidx.compose.ui.text.font.FontWeight
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class PlainTest {
    @Test fun marksAreTakenOutOfAPlainLine() {
        assertEquals("Done and live, PR open", Plain.strip("**Done and live**, PR open"))
        assertEquals("Undo with hq skills revert <name>.", Plain.strip("Undo with `hq skills revert <name>`."))
        assertEquals("see the design and Plan", Plain.strip("see [the design](Design.md) and [[Plan]]"))
        assertEquals("the shown words", Plain.strip("the [[Target|shown words]]"))
        assertEquals("What it is. A tunnel", Plain.strip("1. **What it is.** A tunnel").removePrefix("1. "))
    }

    @Test fun aHeadingMarkIsDropped() {
        assertEquals("Where it stands", Plain.strip("## Where it stands"))
    }

    @Test fun textWithoutMarksIsUntouched() {
        assertEquals("2 of 3 steps * done", Plain.strip("2 of 3 steps * done"))
    }

    @Test fun aCellKeepsBoldAsStyleNotAsStars() {
        val cell = Plain.cell("**Done**, open")
        assertEquals("Done, open", cell.text)
        val bold = cell.spanStyles.single()
        assertEquals(FontWeight.Bold, bold.item.fontWeight)
        assertEquals(0, bold.start)
        assertEquals(4, bold.end)
    }

    @Test fun aCellShowsOnlyTheWordsOfALink() {
        assertEquals("the design", Plain.cell("[the design](Design.md)").text)
        assertTrue(Plain.cell("[the design](Design.md)").spanStyles.isEmpty())
    }

    @Test fun aFencedBlockKeepsItsLinesWithoutTheFences() {
        assertEquals("Run this:\nhq net init", Plain.strip("Run this:\n```bash\nhq net init\n```"))
    }

    @Test fun anImageShowsItsAltText() {
        assertEquals("diagram", Plain.strip("![diagram](x.png)"))
    }

    @Test fun codeNeverSpansLines() {
        assertEquals("a ` b\nc ` d", Plain.strip("a ` b\nc ` d"))
    }

    @Test fun aSnippetWithLineBreaksIsOneLine() {
        assertEquals("reuse what is already built ...", Plain.oneLine("reuse what is\n  already built\n..."))
        assertEquals("a tunnel inside the app", Plain.oneLine("**a** tunnel\ninside the app"))
    }
}
