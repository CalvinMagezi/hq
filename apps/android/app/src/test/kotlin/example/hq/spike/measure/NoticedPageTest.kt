package example.hq.spike.measure

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import example.hq.spike.R
import example.hq.spike.ui.settings.NoticedPage
import example.hq.spike.ui.theme.HqTheme
import java.io.File
import java.nio.file.Files
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w390dp-h1200dp-xxhdpi")
class NoticedPageTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val file = File(Files.createTempDirectory("noticed").toFile().also { it.deleteOnExit() }, Counters.FILE)

    private fun text(id: Int, vararg args: Any) = RuntimeEnvironment.getApplication().getString(id, *args)

    private fun counters(): Counters {
        Counters(file, "0.9.0") { "2026-10-20" }.bump(Counter.NoticePosted)
        return Counters(file, "0.9.1") { "2026-11-03" }
    }

    @Test fun listsEveryCounterTheBoardNamesWithTheIntroAndBothColumns() {
        rule.setContent { HqTheme { NoticedPage(counters()) } }
        rule.onNodeWithText(text(R.string.noticed_intro)).assertExists()
        rule.onNodeWithText(text(R.string.noticed_since, "3 Nov")).assertExists()
        rule.onNodeWithText("0.9.1").assertExists()
        rule.onNodeWithText("0.9.0").assertExists()
        Noticed.lines.forEach { rule.onNodeWithText(text(it.label)).assertExists() }
        rule.onNodeWithText(text(R.string.noticed_foot)).assertExists()
    }

    @Test fun tappingALineSaysWhatItCountsAndDoneClosesIt() {
        rule.setContent { HqTheme { NoticedPage(counters()) } }
        rule.onNodeWithText(text(R.string.noticed_label_task_discarded)).performScrollTo().performClick()
        rule.onNodeWithText(text(R.string.noticed_what_undo_task_discarded)).assertExists()
        rule.onNodeWithText(text(R.string.noticed_only_two)).assertExists()
        rule.onNodeWithText(text(R.string.noticed_done)).performClick()
        rule.onNodeWithText(text(R.string.noticed_only_two)).assertDoesNotExist()
    }

    @Test fun theSwitchStopsCountingAndSaysSoAtTheTop() {
        val c = counters()
        rule.setContent { HqTheme { NoticedPage(c) } }
        rule.onNodeWithText(text(R.string.noticed_switch_on)).performScrollTo().performClick()
        rule.waitForIdle()
        assertFalse(c.snapshot().on)
        rule.onNodeWithText(text(R.string.noticed_switch_off)).assertExists()
        rule.onNodeWithText(text(R.string.noticed_off_note, "3 Nov")).assertExists()
    }

    @Test fun clearAsksFirstAndKeepThemLeavesTheNumbers() {
        val c = counters()
        rule.setContent { HqTheme { NoticedPage(c) } }
        rule.onNodeWithText(text(R.string.noticed_clear)).performScrollTo().performClick()
        rule.onNodeWithText(text(R.string.noticed_clear_body)).assertExists()
        rule.onNodeWithText(text(R.string.noticed_clear_keep)).performClick()
        assertEquals("0.9.0", c.snapshot().previous?.version)
        rule.onNodeWithText(text(R.string.noticed_clear_body)).assertDoesNotExist()
    }

    @Test fun clearNumbersZeroesBothColumns() {
        val c = counters()
        rule.setContent { HqTheme { NoticedPage(c) } }
        rule.onNodeWithText(text(R.string.noticed_clear)).performScrollTo().performClick()
        rule.onNodeWithText(text(R.string.noticed_clear_confirm)).performClick()
        rule.waitForIdle()
        assertTrue(c.snapshot().previous == null)
        assertEquals(0, c.snapshot().current.slot(Counter.NoticePosted))
    }

    @Test @Config(sdk = [31], qualifiers = "w390dp-h1200dp-xxhdpi") fun beforeAndroid13TheNotificationLinesSayNotAsked() {
        rule.setContent { HqTheme { NoticedPage(counters()) } }
        rule.onAllNodesWithTextCount(text(R.string.noticed_not_asked), 3)
    }

    private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.onAllNodesWithTextCount(t: String, n: Int) {
        assertEquals(n, onAllNodes(androidx.compose.ui.test.hasText(t)).fetchSemanticsNodes().size)
    }
}
