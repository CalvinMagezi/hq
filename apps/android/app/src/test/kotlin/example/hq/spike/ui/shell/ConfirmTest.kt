package example.hq.spike.ui.shell

import android.content.Context
import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.click
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.unit.Density
import androidx.test.core.app.ApplicationProvider
import example.hq.spike.ui.theme.HqTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w390dp-h800dp-xxhdpi")
class ConfirmTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
    private val context: Context = ApplicationProvider.getApplicationContext()

    private val words = ConfirmCopy.removeDevice(context, "Work laptop")

    private fun show(scale: Float = 1f, busy: Boolean = false, error: String? = null, onKeep: () -> Unit = {}, onAct: () -> Unit = {}) = rule.setContent {
        CompositionLocalProvider(LocalDensity provides Density(LocalDensity.current.density, scale)) {
            HqTheme { ConfirmDialog(words, onKeep, onAct, busy = busy, error = error) }
        }
    }

    private fun allRed() = listOf(
        ConfirmCopy.stopHelper(context, "Writer"), ConfirmCopy.removeDevice(context, "Work laptop"), ConfirmCopy.signOutDevice(context, "Work laptop"),
        ConfirmCopy.signOutAll(context), ConfirmCopy.disconnectApp(context, "Telegram"), ConfirmCopy.unlinkAccount(context, "Claude"),
        ConfirmCopy.disconnectServer(context, "hq.example.com"), ConfirmCopy.removeKey(context, "Openrouter", only = false), ConfirmCopy.removeKey(context, "Openrouter", only = true),
        ConfirmCopy.leaveNote(context, "Ideas"),
    )

    @Test fun everyRedActionNamesTheTitleVerb() {
        allRed().forEach { w -> assertTrue("${w.title} -> ${w.act}", ConfirmRules.actMatchesTitle(w.title, w.act)) }
    }

    @Test fun noRedWordsAreAYesOrOk() {
        listOf("Yes", "OK", "Continue", "Confirm").forEach { assertFalse(ConfirmRules.actMatchesTitle("Remove it?", it)) }
    }

    @Test fun theWordsAreExact() {
        assertEquals("Remove device", words.act)
        assertEquals("Work laptop removed.", context.getString(example.hq.spike.R.string.remove_device_done, "Work laptop"))
        assertEquals(ConfirmKind.Red, words.kind)
        assertEquals(ConfirmKind.Ask, ConfirmCopy.bulkDone(context, 3).kind)
    }

    @Test fun stacksForALongLabelOrLargeText() {
        assertFalse(ConfirmRules.stacks("Keep", "Remove", 1f))
        assertTrue(ConfirmRules.stacks("Keep", "Remove", 1.5f))
        assertTrue(ConfirmRules.stacks("Keep it connected", "Disconnect everything", 1f))
    }

    @Test fun safeIsLeftOfTheActionAtNormalSize() {
        show()
        val keep = rule.onNodeWithTag("confirm-keep").getUnclippedBoundsInRoot()
        val act = rule.onNodeWithTag("confirm-act").getUnclippedBoundsInRoot()
        assertTrue(keep.right <= act.left)
        assertEquals(keep.top, act.top)
    }

    @Test fun safeIsAboveTheActionAtLargeText() {
        show(scale = 1.5f)
        val keep = rule.onNodeWithTag("confirm-keep").getUnclippedBoundsInRoot()
        val act = rule.onNodeWithTag("confirm-act").getUnclippedBoundsInRoot()
        assertTrue(keep.bottom <= act.top)
    }

    @Test fun theScrimChoosesTheSafeSide() {
        var kept = 0
        var acted = 0
        show(onKeep = { kept++ }, onAct = { acted++ })
        rule.onNodeWithTag("confirm-scrim").performTouchInput { click(Offset(4f, 4f)) }
        assertEquals(1, kept)
        assertEquals(0, acted)
    }

    @Test fun theBackGestureChoosesTheSafeSide() {
        var kept = 0
        var acted = 0
        show(onKeep = { kept++ }, onAct = { acted++ })
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitForIdle()
        assertEquals(1, kept)
        assertEquals(0, acted)
    }

    @Test fun theActionButtonActs() {
        var acted = 0
        show(onAct = { acted++ })
        rule.onNodeWithTag("confirm-act").performClick()
        assertEquals(1, acted)
    }

    @Test fun busyTurnsBothButtonsOffAndSaysWhat() {
        show(busy = true)
        rule.onNodeWithTag("confirm-act").assertIsNotEnabled()
        rule.onNodeWithTag("confirm-keep").assertIsNotEnabled()
        rule.onNodeWithText(words.busy).assertIsDisplayed()
    }

    @Test fun aFailureStaysOnTheDialogWithTheActionUnchanged() {
        show(error = "Couldn't remove it.")
        rule.onNodeWithTag("confirm-error", useUnmergedTree = true).assertIsDisplayed()
        rule.onNodeWithText(words.act).assertIsDisplayed()
    }
}
