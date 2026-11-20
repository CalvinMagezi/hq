package example.hq.spike.data

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The slides, comments and doc-tool wire shapes and the pure logic over them. */
class SuiteSlidesWireTest {
    private val cardsJson = """{"revision":"r1","title":"Deck","cards":[
        {"index":0,"title":"Results","hash":"aaaaaaaaaaaa","blocks":3,"layout":"content","transition":"fade slow","background":"#1F3A5F","color":null,"notes":["Say this first."]},
        {"index":1,"title":"Plan","hash":"bbbbbbbbbbbb","blocks":2,"layout":"two-column","transition":null,"background":null,"color":"#FFFFFF","notes":[]}]}"""

    @Test fun cardsParse() {
        val cards = CardsWire.list(JSONObject(cardsJson))
        assertEquals(2, cards.size)
        assertEquals(CardRow(0, "Results", "aaaaaaaaaaaa", 3, "content", "fade slow", "#1F3A5F", null, listOf("Say this first.")), cards[0])
        assertEquals("two-column", cards[1].layout)
        assertNull(cards[1].transition)
        assertEquals("#FFFFFF", cards[1].color)
    }

    @Test fun cardsWithNothingAreEmpty() {
        assertTrue(CardsWire.list(JSONObject("{}")).isEmpty())
    }

    @Test fun commentsParseWithOpenAndResolved() {
        val o = JSONObject(
            """{"revision":"r","comments":[
            {"index":4,"hash":"c1","on":"b1","by":"Phone","at":"2026-11-20","state":"open","text":"Is this final?","orphaned":false,"anchor_preview":"Revenue"},
            {"index":6,"hash":"c2","on":"b2","state":"resolved","text":"Done","orphaned":true}]}""",
        )
        val list = CommentsWire.list(o)
        assertEquals(CommentRow(4, "c1", "b1", "Phone", "2026-11-20", true, "Is this final?", false, "Revenue"), list[0])
        assertFalse(list[1].open)
        assertTrue(list[1].orphaned)
        assertNull(list[1].by)
        assertNull(list[1].anchor)
    }

    @Test fun noteChangedFrameParses() {
        assertEquals(ChatEvent.NoteEdited("Notebooks/Deck.md", "abc"), ChatEvent.parse("""{"type":"note_changed","path":"Notebooks/Deck.md","hash":"abc"}"""))
        assertNull(ChatEvent.parse("""{"type":"note_changed"}"""))
    }

    private fun block(i: Int, kind: String, src: String) = BlockRow(i, "h$i", kind, src)

    private val deck = listOf(
        block(0, "heading 1", "# Board review"),
        block(1, "paragraph", "Prepared for the board."),
        block(2, "heading 2", "## Results"),
        block(3, "html", "<!-- transition: fade -->"),
        block(4, "list", "- Revenue up"),
        block(5, "heading 2", "## Plan"),
        block(6, "html", "<!-- notes: Say this first. -->"),
        block(7, "paragraph", "Hire.\nTrain."),
        block(8, "comment", "```comment on=h7 state=open\nCheck\n```"),
    )

    @Test fun cardBodyHidesCommentsAndCarriesThemBack() {
        val first = SlideBodies.bodyOf(deck, 0)!!
        assertEquals("- Revenue up", first.text)
        assertEquals(listOf("<!-- transition: fade -->"), first.kept)
        assertEquals("New text\n\n<!-- transition: fade -->", first.markdown("  New text "))
        val second = SlideBodies.bodyOf(deck, 1)!!
        assertEquals("Hire.\nTrain.", second.text)
        assertEquals(2, second.kept.size)
    }

    @Test fun missingCardHasNoBody() {
        assertNull(SlideBodies.bodyOf(deck, 2))
        assertEquals("", SlideBodies.preview(null, 3))
        assertEquals("Hire.\nTrain.", SlideBodies.preview(SlideBodies.bodyOf(deck, 1), 3))
        assertEquals("Hire.", SlideBodies.preview(SlideBodies.bodyOf(deck, 1), 1))
    }

    @Test fun transitionKindDropsTheSpeed() {
        assertEquals("fade", CardStyles.transitionKind("fade slow"))
        assertEquals("none", CardStyles.transitionKind("none"))
        assertNull(CardStyles.transitionKind(null))
        assertNull(CardStyles.transitionKind(" "))
    }

    @Test fun suiteKindReadsFrontMatter() {
        assertEquals("slides", suiteKindOf("---\ntitle: T\nsuite_kind: slides\n---\n# T\n"))
        assertEquals("doc", suiteKindOf("---\nsuite_kind: \"Doc\"\n---\n"))
        assertNull(suiteKindOf("# no front matter\nsuite_kind: slides"))
        assertNull(suiteKindOf("---\ntitle: T\n---\n"))
    }

    @Test fun tableMarkdownIsAGridOfEmptyRows() {
        assertEquals("| A | B |\n| --- | --- |\n|   |   |\n|   |   |", DocTools.tableMarkdown(listOf("A", "B"), 2))
    }

    @Test fun findReplacePlansOnlyProseBlocks() {
        val blocks = listOf(
            block(0, "paragraph", "Sugar and sugar"),
            block(1, "code (rust)", "let Sugar = 1;"),
            block(2, "table", "| Sugar |\n|---|\n| 4 |"),
            block(3, "heading 2", "## Salt"),
            block(4, "sheet", "| Sugar |"),
        )
        val plan = DocTools.plan(blocks, "Sugar", "Honey")
        assertEquals(listOf(0, 2), plan.map { it.row.index })
        assertEquals("Honey and sugar", plan[0].markdown)
        assertEquals(2, DocTools.occurrences(blocks, "Sugar"))
        assertTrue(DocTools.plan(blocks, "", "x").isEmpty())
    }

    @Test fun countDoesNotOverlap() {
        assertEquals(1, DocTools.count("aaa", "aa"))
        assertEquals(0, DocTools.count("abc", ""))
    }
}
