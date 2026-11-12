package example.hq.spike.ui.startup

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import example.hq.spike.ui.theme.HqTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w390dp-h900dp-xxhdpi")
class StartStepsTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun theFirstLaunchCoverListsTheFourStepsCountsAndOffersCapture() {
        var captured = 0
        // The step pulse never settles, so time is advanced by hand.
        rule.mainClock.autoAdvance = false
        rule.setContent { HqTheme { StartingScreen(3, 12) { captured++ } } }
        rule.mainClock.advanceTimeBy(200)
        rule.onNodeWithText("Starting HQ").assertExists()
        rule.onNodeWithText("Step 3 of 4 \u00b7 12 s").assertExists()
        rule.onNodeWithContentDescription("Step 1 of 4, Starting on this phone").assertExists()
        rule.onNodeWithContentDescription("Step 3 of 4, Opening your Vault and tasks").assertExists()
        rule.onNodeWithContentDescription("Step 4 of 4, Almost ready").assertExists()
        rule.onNodeWithText("Capture").performClick()
        assertEquals(1, captured)
    }
}
