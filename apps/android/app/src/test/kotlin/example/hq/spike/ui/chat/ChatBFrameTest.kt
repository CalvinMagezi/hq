package example.hq.spike.ui.chat

import android.content.Context
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import example.hq.spike.R
import example.hq.spike.ui.theme.HqTheme
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Frame and catalogue-string tests at 200% font scale and narrow phone width for W3b chat additions. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w390dp-h800dp-xxhdpi")
class ChatBFrameTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private fun layoutOf(text: String): TextLayoutResult {
        val results = mutableListOf<TextLayoutResult>()
        rule.onAllNodes(hasText(text), useUnmergedTree = true).fetchSemanticsNodes()
            .forEach { it.config.getOrNull(SemanticsActions.GetTextLayoutResult)?.action?.invoke(results) }
        return results.first { it.layoutInput.text.text == text }
    }

    private fun cutOff(text: String, widthDp: Int): Boolean {
        val l = layoutOf(text)
        val ellipsized = (0 until l.lineCount).any { l.isLineEllipsized(it) }
        val right = rule.onAllNodes(hasText(text), useUnmergedTree = true).fetchSemanticsNodes()
            .first { it.config.getOrNull(SemanticsActions.GetTextLayoutResult) != null }.boundsInRoot.right
        val spills = right > with(rule.density) { widthDp.dp.toPx() }
        return ellipsized || spills
    }

    @Test fun everyFollowUpChipCatalogueStringIsAtMostFortyCharsWithoutEmDash() {
        val ctx = ApplicationProvider.getApplicationContext<Context>()
        ReplyToolKind.entries.forEach { kind ->
            val ids = FollowUpChips.TABLE[kind].orEmpty()
            assertTrue("Expected 1..3 chips for $kind", ids.size in 1..FollowUpChips.MAX)
            ids.forEach { resId ->
                val text = ctx.getString(resId)
                assertTrue("Chip too long (${text.length}): $text", text.isNotBlank() && text.length <= FollowUpChips.LABEL_MAX)
                assertFalse("Em-dash not allowed in chip: $text", text.contains('—'))
            }
        }
    }

    @Test fun offlineProblemBarNeverPromisesAQueue() {
        val ctx = ApplicationProvider.getApplicationContext<Context>()
        val text = ctx.getString(R.string.chat_problem_offline)
        assertFalse("Offline bar must not claim a queue: $text", text.contains("queue", ignoreCase = true))
        assertFalse("Offline bar must not claim queued sends wait: $text", text.contains("waiting", ignoreCase = true))
    }

    @Test fun problemBarWrapsAtTwoHundredPercentAndAnnouncesPolitely() {
        val widthDp = 360
        rule.setContent {
            val d = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(d.density, 2.0f)) {
                HqTheme {
                    Box(Modifier.width(widthDp.dp)) {
                        ProblemBarView(ProblemBar.State.SignedOut, onSignIn = {})
                    }
                }
            }
        }
        rule.waitForIdle()
        assertFalse(cutOff("Signed out of your server HQ.", widthDp))
        assertFalse(cutOff("Sign in", widthDp))
        val hasPolite = rule.onAllNodes(hasText("Signed out of your server HQ."), useUnmergedTree = false)
            .fetchSemanticsNodes()
            .any { it.config.getOrNull(SemanticsProperties.LiveRegion) != null }
        assertTrue(hasPolite)
    }

    @Test fun followUpChipsWrapCleanlyAtTwoHundredPercent() {
        val widthDp = 360
        val ctx = ApplicationProvider.getApplicationContext<Context>()
        val chipIds = FollowUpChips.TABLE[ReplyToolKind.NoteSaved].orEmpty()
        rule.setContent {
            val d = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(d.density, 2.0f)) {
                HqTheme {
                    Box(Modifier.width(widthDp.dp)) {
                        FollowUpChipsSlot(chipIds, "r1", onPick = {})
                    }
                }
            }
        }
        rule.waitForIdle()
        chipIds.forEach { resId ->
            val chip = ctx.getString(resId)
            assertFalse("Chip cut off at 200%: $chip", cutOff(chip, widthDp))
        }
    }

    @Test fun notSentStateWrapsCleanlyAtTwoHundredPercent() {
        val widthDp = 360
        rule.setContent {
            val d = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(d.density, 2.0f)) {
                HqTheme {
                    Box(Modifier.width(widthDp.dp)) {
                        UserBubble("Hello HQ", sending = false, sendStatus = SendStatus.NotSent, onRetrySend = {})
                    }
                }
            }
        }
        rule.waitForIdle()
        assertFalse(cutOff("Not sent", widthDp))
        assertFalse(cutOff("Try again", widthDp))
    }
}
