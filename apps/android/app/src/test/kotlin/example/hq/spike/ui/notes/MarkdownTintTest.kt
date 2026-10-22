package example.hq.spike.ui.notes

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import org.junit.Assert.assertEquals
import org.junit.Test

class MarkdownTintTest {
    private fun tinted(text: String) = MarkdownTint.spans(text).map { text.substring(it.start, it.end) to it.kind }

    private val sample = """
        |## Workstream C
        |A native view instead of the WebView.
        |
        |- **Chat** first, then approvals
        |- Tasks and quick capture
        |- [[Android-On-Device-Assessment]]
        |
        |- [x] Draw the note screens
        |- [ ] Write the contract
    """.trimMargin()

    @Test fun theBoardsSampleIsTintedAsDrawn() {
        assertEquals(
            listOf(
                "## " to TintKind.Mark, "Workstream C" to TintKind.Heading,
                "- " to TintKind.Mark, "**" to TintKind.Mark, "**" to TintKind.Mark,
                "- " to TintKind.Mark,
                "- " to TintKind.Mark, "[[Android-On-Device-Assessment]]" to TintKind.Wiki,
                "- [x] " to TintKind.Mark,
                "- [ ] " to TintKind.Mark,
            ),
            tinted(sample),
        )
    }

    @Test fun everyHeadingLevelIsTinted() {
        for (level in 1..6) assertEquals("#".repeat(level) + " " to TintKind.Mark, tinted("#".repeat(level) + " Title").first())
        assertEquals(emptyList<Pair<String, TintKind>>(), tinted("####### seven"))
        assertEquals(emptyList<Pair<String, TintKind>>(), tinted("#tag"))
    }

    @Test fun singleStarsAroundItalicTextAreMarks() {
        assertEquals(listOf("*" to TintKind.Mark, "*" to TintKind.Mark), tinted("an *italic* word"))
        assertEquals(emptyList<Pair<String, TintKind>>(), tinted("2 * 3 * 4"))
    }

    @Test fun boldAndItalicOnOneLineAreBothFound() {
        assertEquals(listOf("**", "**", "*", "*"), tinted("**bold** and *it*").map { it.first })
    }

    @Test fun anUnclosedMarkIsLeftAlone() {
        assertEquals(emptyList<Pair<String, TintKind>>(), tinted("**never closed and [[also not"))
    }

    @Test fun spansLieInsideTheTextWhateverIsTyped() {
        val text = "# a\n\n- [ ]\n**\n***\n[[]]\n[[x]]\n* \n"
        for (s in MarkdownTint.spans(text)) assertEquals(true, s.start in 0..s.end && s.end <= text.length)
    }

    @Test fun theTransformationNeverChangesTheTextOrTheCursor() {
        val text = "## Heading with **bold** and [[a link]]\n- item"
        val out = MarkdownTransformation.filter(androidx.compose.ui.text.AnnotatedString(text))
        assertEquals(text, out.text.text)
        for (i in 0..text.length) assertEquals(i, out.offsetMapping.originalToTransformed(i))
    }

    @Test fun aHugeTextIsLeftPlain() {
        val text = "- x\n".repeat(MarkdownTint.MAX_CHARS / 4 + 10)
        assertEquals(0, MarkdownTransformation.filter(androidx.compose.ui.text.AnnotatedString(text)).text.spanStyles.size)
    }

    private fun write(path: String?, title: String) = WriteState(path, "Notebooks/Inbox", title, TextFieldValue("x", TextRange(1)), original = "")

    @Test fun theLeaveDialogNamesTheNote() {
        assertEquals("You edited Mobile-First-Roadmap and have not saved. Your text is kept on this phone unless you discard it.", write("Notebooks/Projects/Mobile-First-Roadmap.md", "Mobile-First-Roadmap").leaveBody)
        assertEquals("You edited Mobile-First-Roadmap and have not saved. Your text is kept on this phone unless you discard it.", write("Notebooks/Projects/Mobile-First-Roadmap.md", "").leaveBody)
        assertEquals("You wrote Ideas and have not saved it. Your text is kept on this phone unless you discard it.", write(null, "Ideas").leaveBody)
        assertEquals("You wrote a note and have not saved it. Your text is kept on this phone unless you discard it.", write(null, "").leaveBody)
    }

    @Test fun aLongLineIsLeftPlain() {
        val line = "**x ".repeat(MarkdownTint.MAX_LINE)
        assertEquals(0, MarkdownTint.spans(line).size)
        assertEquals(2, MarkdownTint.spans("**x**").size)
    }

    @Test fun aCodeFenceIsNotTinted() {
        assertEquals(emptyList<Pair<String, TintKind>>(), tinted("```\n# not a heading\n- not a list\n```"))
        assertEquals("## " to TintKind.Mark, tinted("```\ncode\n```\n## Real").first())
    }

    @Test fun aNewNoteWithOnlyATitleDoesNotPromiseItsText() {
        val w = WriteState(null, "Notebooks/Inbox", "Ideas", TextFieldValue("", TextRange(0)), original = "")
        assertEquals("You wrote Ideas and have not saved it.", w.leaveBody)
    }
}
