package example.hq.spike.ui.shell

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.hasText
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import example.hq.spike.ui.theme.HqTheme
import example.hq.spike.ui.theme.Mts
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Real composition at phone widths and text sizes: the status word is never the part that gets cut. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w390dp-h800dp-xxhdpi")
class TopBarLayoutTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val statuses = listOf("43 open · Connected", "Starting HQ, step 2 of 4 · 7 s", "HQ is stopped", "Offline")

    private data class Cfg(val title: String, val status: String, val widthDp: Int, val scale: Float)
    private var cfg by mutableStateOf<Cfg?>(null)
    private var started = false

    /** One composition for the whole test; each call changes what it shows. */
    private fun show(title: String, status: String, widthDp: Int, fontScale: Float) {
        cfg = Cfg(title, status, widthDp, fontScale)
        if (!started) {
            started = true
            rule.setContent {
                val c = cfg ?: return@setContent
                val d = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(d.density, c.scale)) {
                    HqTheme { Box(Modifier.width(c.widthDp.dp)) { TopBar(c.title, c.status, Mts.InkMuted, onSearch = {}, onSettings = {}) } }
                }
            }
        }
        rule.waitForIdle()
    }

    private fun layoutOf(text: String): TextLayoutResult {
        val results = mutableListOf<TextLayoutResult>()
        rule.onAllNodes(hasText(text), useUnmergedTree = true).fetchSemanticsNodes()
            .forEach { it.config.getOrNull(SemanticsActions.GetTextLayoutResult)?.action?.invoke(results) }
        return results.first { it.layoutInput.text.text == text }
    }

    /** Cut means an ellipsis on some line, or text running past the bar. A status that wraps onto a second line is whole. */
    private fun cutOff(text: String, widthDp: Int = 0): Boolean {
        val l = layoutOf(text)
        val ellipsized = (0 until l.lineCount).any { l.isLineEllipsized(it) }
        val spills = widthDp > 0 && bounds(text).right > with(rule.density) { widthDp.dp.toPx() }
        return ellipsized || spills
    }

    private fun bounds(text: String) = rule.onAllNodes(hasText(text), useUnmergedTree = true).fetchSemanticsNodes().first { it.config.getOrNull(SemanticsActions.GetTextLayoutResult) != null }.boundsInRoot

    @Test fun theStatusIsNeverCutAtAnyWidthOrTextSize() {
        for (width in listOf(390, 360)) for (scale in listOf(1.0f, 1.3f, 2.0f)) for (status in statuses) {
            show("Tasks", status, width, scale)
            assertFalse("$status at ${width}dp x$scale", cutOff(status, width))
        }
    }

    @Test fun aLongTitleGivesWayToTheStatus() {
        show("A very long place name that cannot fit", "Offline", 360, 1.0f)
        assertFalse(cutOff("Offline"))
        assertTrue(cutOff("A very long place name that cannot fit"))
    }

    @Test fun roomyWidthPutsTheStatusBesideTheTitleAndTightWidthUnderIt() {
        show("Chat", "Connected", 390, 1.0f)
        val title = bounds("Chat")
        val beside = bounds("Connected")
        assertTrue(beside.left >= title.right)
        show("Tasks", "Starting HQ, step 2 of 4 · 7 s", 360, 2.0f)
        val t2 = bounds("Tasks")
        val under = bounds("Starting HQ, step 2 of 4 · 7 s")
        assertTrue(under.top >= t2.bottom)
        assertTrue(under.left >= t2.left)
    }
}
