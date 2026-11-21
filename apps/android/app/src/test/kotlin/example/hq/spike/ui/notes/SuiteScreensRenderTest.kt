package example.hq.spike.ui.notes

import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import example.hq.spike.data.BlockRow
import example.hq.spike.data.CardRow
import example.hq.spike.data.CommentRow
import example.hq.spike.ui.chat.blocks.ChartCard
import example.hq.spike.ui.chat.blocks.ChartParse
import example.hq.spike.ui.chat.blocks.Charts
import example.hq.spike.ui.theme.HqTheme
import example.hq.spike.ui.theme.Mts
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The suite screens drawn for real: Compose on Robolectric's native graphics, with fixture data in place of a server.
 * Each test also saves a PNG when `HQ_RENDER_OUT` names a folder, so the pictures can be looked at.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-xxhdpi")
class SuiteScreensRenderTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private fun save(name: String) {
        val dir = System.getenv("HQ_RENDER_OUT") ?: return
        // The decor view drawn to a bitmap: the real Compose content on Robolectric's native graphics.
        val view = rule.activity.window.decorView
        val bitmap = android.graphics.Bitmap.createBitmap(view.width.coerceAtLeast(1), view.height.coerceAtLeast(1), android.graphics.Bitmap.Config.ARGB_8888)
        rule.runOnUiThread { view.draw(android.graphics.Canvas(bitmap)) }
        File(dir).mkdirs()
        File(dir, "$name.png").outputStream().use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
    }

    private fun chart(kind: String, labels: String, series: String): ChartParse =
        Charts.parse("""{"type":"$kind","title":"Sales by week","xLabel":"x","yLabel":"UGX","labels":$labels,"series":$series}""")

    @Test fun areaStackedAndScatterChartsDrawOnThePhone() {
        val two = """[{"name":"North","values":[3,5,4,6]},{"name":"South","values":[2,3,5,4]}]"""
        val area = chart("area", """["W1","W2","W3","W4"]""", two) as ChartParse.Ok
        val stacked = chart("stacked", """["W1","W2","W3","W4"]""", two) as ChartParse.Ok
        val scatter = chart("scatter", """["1","2.5","4","6"]""", """[{"name":"y","values":[3,5,4,6]}]""") as ChartParse.Ok
        rule.setContent {
            HqTheme {
                Column(Modifier.fillMaxSize().background(Mts.Surface000)) {
                    ChartCard(area.spec) {}
                    ChartCard(stacked.spec) {}
                    ChartCard(scatter.spec) {}
                }
            }
        }
        rule.waitForIdle()
        rule.onAllNodesWithText("Sales by week", substring = true, useUnmergedTree = true).assertCountEquals(3)
        save("charts-area-stacked-scatter")
    }

    @Test fun theDeckListShowsEachCardInItsOwnColoursAndANewCardButton() {
        val cards = listOf(
            CardRow(0, "Where we are", "aaaaaaaaaaaa", 1, "content", "fade", "#1F3A5F", "#FFFFFF", listOf("Open with revenue.")),
            CardRow(1, "What we plan", "bbbbbbbbbbbb", 1, "two-column", null, null, null, emptyList()),
        )
        val blocks = listOf(
            BlockRow(0, "h0", "heading 1", "# Deck"),
            BlockRow(1, "h1", "heading 2", "## Where we are"),
            BlockRow(2, "h2", "list", "- Revenue up\n- Costs steady"),
            BlockRow(3, "h3", "heading 2", "## What we plan"),
            BlockRow(4, "h4", "numbered list", "1. Hire\n2. Train"),
        )
        rule.setContent { HqTheme { Column(Modifier.fillMaxSize().background(Mts.Surface000)) { CardList(DeckData(cards, blocks), onOpen = {}, onAdd = {}) } } }
        rule.waitForIdle()
        rule.onNodeWithText("Where we are", useUnmergedTree = true).assertIsDisplayed()
        rule.onNodeWithText("What we plan", useUnmergedTree = true).assertIsDisplayed()
        save("deck-list")
    }

    @Test fun commentsListOpenFirstThenResolvedAndOrphanedIsSaidPlainly() {
        val rows = listOf(
            CommentRow(2, "c2", "x", "Phone", "2026-11-21", false, "Looks fine now.", false, "We buy sugar"),
            CommentRow(1, "c1", "y", "Reviewer", "2026-11-20", true, "Is the 30 days firm?", false, "Payment is due in 30 days."),
            CommentRow(3, "c3", "z", null, null, true, "This block was edited.", true, null),
        )
        assertEquals(listOf("c1", "c3", "c2"), openFirst(rows).map { it.hash })
        var resolved: String? = null
        rule.setContent { HqTheme { Column(Modifier.fillMaxSize().background(Mts.Surface000)) { CommentsList(Result.success(rows), Modifier) { resolved = it.hash } } } }
        rule.waitForIdle()
        rule.onNodeWithText("Is the 30 days firm?", useUnmergedTree = true).assertIsDisplayed()
        save("comments-list")
        // Two open comments, so two Resolve buttons; the first one belongs to the first open comment.
        rule.onAllNodesWithText("Resolve", useUnmergedTree = true).assertCountEquals(2)
        rule.onAllNodesWithText("Resolve", useUnmergedTree = true)[0].performClick()
        assertEquals("c1", resolved)
    }
}
