package example.hq.spike.data

import example.hq.spike.ui.chat.blocks.ChartParse
import example.hq.spike.ui.chat.blocks.Charts
import java.io.File
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

/**
 * The phone's suite calls against a real running hq. A diagnostic, not a gate: it skips unless `HQ_LIVE_URL` names a
 * server (no token needed on loopback) and `HQ_LIVE_VAULT` is that server's vault folder, which holds the notes
 * `Live/deck.md`, `Live/stock.md` and `Live/sales.md` (see docs/suite/README.md). Run it with
 * `HQ_LIVE_URL=http://127.0.0.1:5690 HQ_LIVE_VAULT=/path ./gradlew :app:testDebugUnitTest --tests '*SuiteLiveTest*'`.
 */
class SuiteLiveTest {
    private val url = System.getenv("HQ_LIVE_URL")
    private val vault = System.getenv("HQ_LIVE_VAULT")
    private fun client() = HqClient({ "" }, url)
    private fun note(name: String) = File(vault, "Live/$name.md").readText()

    @Test fun aDeckIsReadEditedMovedAndDeletedThroughTheClient() = runBlocking {
        assumeTrue(url != null && vault != null)
        val c = client()
        val path = "Live/deck.md"
        val cards = c.suiteCards(path)
        assertEquals(listOf("One", "Two", "Three"), cards.map { it.title })
        c.suiteSaveCard(path, cards[0], CardChange("One", "- a", "Say hello first.", "two-column", "fade"), textChanged = false)
        var text = note("deck")
        assertTrue(text, text.contains("<!-- notes: Say hello first. -->") && text.contains("layout: two-column") && text.contains("transition: fade"))
        c.suiteAddCard(path, "Four", "- d", after = 2, hash = c.suiteCards(path)[2].hash)
        assertEquals(listOf("One", "Two", "Three", "Four"), c.suiteCards(path).map { it.title })
        c.suiteMoveCard(path, c.suiteCards(path)[3], 0)
        assertEquals("Four", c.suiteCards(path).first().title)
        c.suiteDeleteCard(path, c.suiteCards(path).first())
        assertEquals(listOf("One", "Two", "Three"), c.suiteCards(path).map { it.title })
        assertEquals("one frontmatter block", 2, Regex("^---$", RegexOption.MULTILINE).findAll(note("deck")).count())
    }

    @Test fun aCommentIsAddedListedAndResolvedThroughTheClient() = runBlocking {
        assumeTrue(url != null && vault != null)
        val c = client()
        val path = "Live/deck.md"
        val block = c.suiteBlocks(path).first { it.kind == "paragraph" || it.kind.startsWith("heading 2") }
        c.suiteAddComment(path, block.hash, "Is this final?", "Phone")
        val listed = c.suiteComments(path)
        assertTrue(listed.any { it.text == "Is this final?" && it.by == "Phone" && it.open })
        c.suiteResolveComment(path, listed.first { it.text == "Is this final?" })
        assertTrue(c.suiteComments(path).none { it.open && it.text == "Is this final?" })
    }

    @Test fun aRowAndAColumnAreAddedAndTheFormulasFollow() = runBlocking {
        assumeTrue(url != null && vault != null)
        val c = client()
        val path = "Live/stock.md"
        c.suiteAddRow(path, 0, 2)
        c.suiteAddColumn(path, 0, "Notes")
        val text = note("stock")
        assertTrue(text, text.contains("| item") && text.contains("Notes"))
        // The total's range grew with the inserted row (E2:E4 became E2:E5), so the sheet still adds every item.
        assertTrue(text, text.contains("=SUM(E2:E5)"))
        val view = c.suiteSheet(path, 0)
        assertTrue(view.rows.isNotEmpty())
    }

    @Test fun theNewChartKindsComeBackFromHqAndTheAndroidParserAcceptsThem() = runBlocking {
        assumeTrue(url != null && vault != null)
        val c = client()
        val path = "Live/sales.md"
        for (n in 0..2) {
            val json = c.suitePlot(path, n)
            val parsed = Charts.parse(json)
            assertTrue("plot $n: $json -> $parsed", parsed is ChartParse.Ok)
        }
        val kinds = (0..2).map { (Charts.parse(c.suitePlot(path, it)) as ChartParse.Ok).spec.type.name }
        assertEquals(listOf("Area", "Stacked", "Scatter"), kinds)
    }
}
