package example.hq.spike.ui.shell

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Text
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import example.hq.spike.ui.theme.HqTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.test.hasText
import androidx.compose.ui.unit.dp
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class AttentionHostTest {
    private companion object { const val FRAME_MS = 16L }

    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun twoBannersPostedTogetherShowOnlyTheBetterOne() {
        val budget = AttentionBudget()
        rule.setContent {
            HqTheme {
                CompositionLocalProvider(LocalAttention provides budget) {
                    AttentionPost(AttentionKind.UpdateLine, "update") { Text("A new version is ready") }
                    AttentionPost(AttentionKind.ConnectionBanner, "connection") { Text("HQ is starting") }
                    Column { BannerSlot(budget) }
                }
            }
        }
        rule.onNodeWithText("HQ is starting").assertIsDisplayed()
        assertEquals(0, rule.onAllNodes(androidx.compose.ui.test.hasText("A new version is ready")).fetchSemanticsNodes().size)
    }

    @Test fun aSnackbarWithoutAShellDrawsInPlace() {
        rule.setContent { HqTheme { UndoSnackbar("Chat archived", {}) } }
        rule.onNodeWithText("Chat archived").assertIsDisplayed()
    }

    @Test fun anUndoSnackbarGoesThroughTheShell() {
        val budget = AttentionBudget()
        rule.setContent {
            HqTheme {
                CompositionLocalProvider(LocalAttention provides budget) {
                    UndoSnackbar("Chat archived", {})
                    SnackbarSlot(budget, 0.dp, 0.dp)
                }
            }
        }
        rule.onNodeWithText("Chat archived").assertIsDisplayed()
        assertEquals(1, budget.posted().size)
    }

    private fun slotContent(budget: AttentionBudget, content: @Composable () -> Unit = {}) = rule.setContent {
        HqTheme { CompositionLocalProvider(LocalAttention provides budget) { content(); SnackbarSlot(budget, 0.dp, 0.dp) } }
    }

    /** Moves the test clock and lets the frames that follow run, since the clock is held by hand. */
    private fun tick(ms: Long) { rule.mainClock.advanceTimeBy(ms); rule.mainClock.advanceTimeBy(FRAME_MS * 3) }

    /** State written from the test thread must be applied before the frames that should show it. */
    private fun onUi(block: () -> Unit) { rule.runOnUiThread { block(); androidx.compose.runtime.snapshots.Snapshot.sendApplyNotifications() } }

    private fun has(text: String) = rule.onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty()

    @Test fun theShellClosesAPlainSnackbarAfterFourSecondsAndTellsItsPoster() {
        val budget = AttentionBudget()
        var told = 0
        rule.mainClock.autoAdvance = false
        slotContent(budget) { AttentionPost(AttentionKind.Snackbar, "a", onExpire = { told++ }) { Text("Saved") } }
        tick(100)
        assertTrue(has("Saved"))
        tick(AttentionBudget.PLAIN_SNACKBAR_MS - 200)
        assertTrue(has("Saved"))
        tick(300)
        assertFalse(has("Saved"))
        assertEquals(1, told)
    }

    @Test fun aSecondSnackbarKeepsItsFullTimeFromWhenItIsShown() {
        val budget = AttentionBudget()
        rule.mainClock.autoAdvance = false
        slotContent(budget)
        onUi { budget.post(AttentionItem(AttentionKind.Snackbar, "one") { Text("First") }) }
        tick(100)
        tick(3_000)
        onUi { budget.post(AttentionItem(AttentionKind.Snackbar, "two") { Text("Second") }) }
        tick(1_100)
        assertTrue("second shows once the first has gone", has("Second"))
        tick(AttentionBudget.PLAIN_SNACKBAR_MS - 500)
        assertTrue("second still has most of its time", has("Second"))
        tick(1_000)
        assertFalse(has("Second"))
    }

    @Test fun anUndoSurvivesAThirdSnackbarAndStaysTenSeconds() {
        val budget = AttentionBudget()
        rule.mainClock.autoAdvance = false
        slotContent(budget)
        onUi { budget.post(AttentionItem(AttentionKind.Snackbar, "one") { Text("First") }) }
        onUi { budget.post(AttentionItem(AttentionKind.UndoSnackbar, "undo") { Text("Archived") }) }
        onUi { budget.post(AttentionItem(AttentionKind.Snackbar, "three") { Text("Third") }) }
        tick(AttentionBudget.PLAIN_SNACKBAR_MS + 200)
        assertTrue(has("Archived"))
        assertFalse(has("Third"))
        tick(AttentionBudget.UNDO_SNACKBAR_MS - 500)
        assertTrue(has("Archived"))
        tick(1_000)
        assertFalse(has("Archived"))
    }

    @Test fun aPosterThatLeavesTakesItsItemWithIt() {
        val budget = AttentionBudget()
        var on by androidx.compose.runtime.mutableStateOf(true)
        slotContent(budget) { if (on) AttentionPost(AttentionKind.ConnectionBanner, "c") { Text("Offline banner") } }
        assertEquals(1, budget.posted().size)
        on = false
        rule.waitForIdle()
        assertEquals(0, budget.posted().size)
    }

    @Test fun aBannerThatSwapsWordsOnPlaceChangeIsNotALiveRegionButAnAnnouncingOneIs() {
        val budget = AttentionBudget()
        rule.setContent {
            HqTheme { CompositionLocalProvider(LocalAttention provides budget) {
                AttentionPost(AttentionKind.ConnectionBanner, "c", announce = false) { Text("Quiet strip") }
                Column { BannerSlot(budget) }
            } }
        }
        rule.onNodeWithText("Quiet strip").assertIsDisplayed()
        assertEquals(0, rule.onAllNodes(androidx.compose.ui.test.SemanticsMatcher.keyIsDefined(androidx.compose.ui.semantics.SemanticsProperties.LiveRegion)).fetchSemanticsNodes().size)
        onUi { budget.post(AttentionItem(AttentionKind.ConnectionBanner, "c", announce = true) { Text("Loud banner") }) }
        rule.waitForIdle()
        assertEquals(1, rule.onAllNodes(androidx.compose.ui.test.SemanticsMatcher.keyIsDefined(androidx.compose.ui.semantics.SemanticsProperties.LiveRegion)).fetchSemanticsNodes().size)
    }

    @Test fun aGatedBannerDrawsOnlyWhileItWinsTheSlot() {
        val budget = AttentionBudget()
        var connection by androidx.compose.runtime.mutableStateOf(true)
        rule.setContent {
            HqTheme { CompositionLocalProvider(LocalAttention provides budget) {
                if (connection) AttentionPost(AttentionKind.ConnectionBanner, "c") { Text("HQ is starting") }
                AttentionGate(AttentionKind.NotificationsBlocked, "n") { Text("Notifications are off") }
            } }
        }
        assertFalse(has("Notifications are off"))
        connection = false
        rule.waitForIdle()
        assertTrue(has("Notifications are off"))
    }

    @Test fun aSnackbarFromAFullScreenPageSitsAtTheEdgeAndAShellOneAboveTheBar() {
        val budget = AttentionBudget()
        onUi { budget.post(AttentionItem(AttentionKind.Snackbar, "shell") { Text("Shell line") }) }
        rule.setContent { HqTheme { androidx.compose.foundation.layout.Box(androidx.compose.ui.Modifier.fillMaxSize()) { SnackbarSlot(budget, aboveBar = 100.dp, belowBar = 0.dp, androidx.compose.ui.Modifier.align(androidx.compose.ui.Alignment.BottomCenter)) } } }
        val shell = rule.onNodeWithText("Shell line").fetchSemanticsNode().boundsInRoot.bottom
        onUi { budget.clear(AttentionKind.Snackbar, "shell") }
        onUi { budget.post(AttentionItem(AttentionKind.Snackbar, "page", coversBar = true) { Text("Page line") }) }
        rule.waitForIdle()
        val page = rule.onNodeWithText("Page line").fetchSemanticsNode().boundsInRoot.bottom
        assertTrue("page line is lower: $page vs $shell", page > shell + 50)
    }

    @Test fun sayingTheSameThingAgainRestartsItsTime() {
        val budget = AttentionBudget()
        rule.mainClock.autoAdvance = false
        slotContent(budget)
        onUi { budget.say("Copied") }
        tick(3_000)
        onUi { budget.say("Copied") }
        tick(2_000)
        assertTrue("restarted, so still up after 5 s in all", has("Copied"))
        tick(2_500)
        assertFalse(has("Copied"))
    }
}
