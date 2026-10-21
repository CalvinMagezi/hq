package example.hq.spike.ui.chat.blocks

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class MessageBlocksTest {
    @Test fun aChartFenceThatNeverClosedIsShownAsItIsOnceTheReplyIsFinished() {
        val md = "Here it is:\n```chart\n{\"type\":\"bar\",\"labels\":[\"a\""
        assertFalse((parseBlocks(md, streaming = true).last() as Block.ChartBlock).closed)
        assertTrue((parseBlocks(md, streaming = false).last() as Block.ChartBlock).closed)
        assertTrue((parseBlocks("```kotlin\nval x = 1", streaming = false).single() as Block.Code).closed)
    }

    @Test fun theBoardsTableParsesWithRightAlignedColumns() {
        val md = "Three runs finished.\n\n|Run|Result|Time|\n|---|---|---:|\n|Run 1|Passed|2 h 10 m|\n|Run 3|Failed|41 m|\n\nThe latest run:"
        val blocks = parseBlocks(md)
        assertEquals(Block.Paragraph("Three runs finished."), blocks[0])
        val t = blocks[1] as Block.Table
        assertEquals(listOf("Run", "Result", "Time"), t.headers)
        assertEquals(listOf(Align.Start, Align.Start, Align.End), t.aligns)
        assertEquals(listOf("Run 3", "Failed", "41 m"), t.rows[1])
        assertEquals(Block.Paragraph("The latest run:"), blocks[2])
    }

    @Test fun aChartFenceBecomesAChartAndAnUnclosedOneSaysItIsStillArriving() {
        val closed = parseBlocks("```chart\n{\"type\":\"bar\",\"labels\":[\"a\"],\"series\":[{\"name\":\"s\",\"values\":[2]}]}\n```") .single() as Block.ChartBlock
        assertTrue(closed.result is ChartParse.Ok)
        val open = parseBlocks("Here:\n```chart\n{\"type\":\"bar\",\"lab", streaming = true).last() as Block.ChartBlock
        assertFalse(open.closed)
    }

    @Test fun aBadChartKeepsItsSourceAndReason() {
        val b = parseBlocks("```chart\n{\"type\":\"bar\",\"labels\":[\"A\",\"B\",\"C\"],\"series\":[{\"name\":\"Runs\",\"values\":[4,7]}]}\n```").single() as Block.ChartBlock
        assertTrue((b.result as ChartParse.Bad).reason.contains("series 1 needs 3 values"))
        assertTrue(b.source.contains("\"Runs\""))
    }

    @Test fun mermaidPieBecomesAChartAndOtherMermaidStaysCode() {
        assertTrue(parseBlocks("```mermaid\npie\n  \"a\" : 1\n  \"b\" : 3\n```").single() is Block.ChartBlock)
        assertTrue(parseBlocks("```mermaid\ngraph TD\n A-->B\n```").single() is Block.Code)
    }

    @Test fun codeQuoteChecklistListAndImage() {
        val md = "To check:\n\n```bash\nadb devices\n```\n\n> Samsung may undo this.\n\n- [x] Allow background activity\n- [ ] Turn off automatic removal\n\n1. Open Settings\n2. Find HQ\n\n![a diagram](https://example.com/x.png)"
        val b = parseBlocks(md)
        assertEquals(Block.Code("bash", "adb devices", true), b[1])
        assertEquals(Block.Quote("Samsung may undo this."), b[2])
        assertEquals(Block.Checklist(listOf(true to "Allow background activity", false to "Turn off automatic removal")), b[3])
        assertEquals(Block.Bullets(true, listOf("Open Settings", "Find HQ")), b[4])
        assertEquals(Block.Image("a diagram"), b[5])
    }

    @Test fun headingsAndPlainTextKeepTheirOrder() {
        val b = parseBlocks("## Samsung battery steps\n\nDo these four things.\nThen test.")
        assertEquals(Block.Heading(2, "Samsung battery steps"), b[0])
        assertEquals(Block.Paragraph("Do these four things.\nThen test."), b[1])
    }

    @Test fun noteReferencesFollowTheWebRules() {
        assertEquals("Notebooks/Projects/Plan.md", NoteRefs.fromCode("Notebooks/Projects/Plan.md"))
        assertNull(NoteRefs.fromCode("README.md"))
        assertNull(NoteRefs.fromCode("a/b.md and more"))
        assertEquals("Notes/A B.md", NoteRefs.fromLink("Notes/A%20B.md#top"))
        assertEquals("Notes/X.md", NoteRefs.fromLink("vault:Notes/X.md"))
        assertNull(NoteRefs.fromLink("https://example.com/x.md"))
        assertNull(NoteRefs.sanitize("../etc/passwd"))
        assertEquals("Mobile-First-Roadmap" to "Roadmap", NoteRefs.wikilink("Mobile-First-Roadmap|Roadmap"))
        assertEquals("Projects › Agent-HQ", NoteRefs.folderTrail("Notebooks/Projects/Agent-HQ/Plan.md"))
    }
}
