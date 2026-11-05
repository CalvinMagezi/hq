package example.hq.spike.ui.firstrun

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithText
import example.hq.spike.ui.settings.AssistantFacts
import example.hq.spike.ui.theme.HqTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w390dp-h900dp-xxhdpi")
class AssistantCardTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private fun show(f: AssistantFacts) = rule.setContent { HqTheme { AssistantCard(f) } }

    @Test fun whereThePhoneHasNoSettingTheCardIsAbsent() {
        show(AssistantFacts(available = false, held = false, secure = true, lockAllowed = true))
        rule.onAllNodesWithText("Optional").assertCountEquals(0)
        rule.onAllNodesWithText("Set up").assertCountEquals(0)
    }

    @Test fun anOptionalCardOffersSetUp() {
        show(AssistantFacts(available = true, held = false, secure = true, lockAllowed = true))
        rule.onNodeWithText("Use HQ as your phone's assistant").assertIsDisplayed()
        rule.onNodeWithText("Optional").assertIsDisplayed()
        rule.onNodeWithText("Set up").assertIsDisplayed()
    }

    @Test fun onceChosenTheCardSaysSoAndOffersNothing() {
        show(AssistantFacts(available = true, held = true, secure = true, lockAllowed = true))
        rule.onNodeWithText("HQ is your phone's assistant").assertIsDisplayed()
        rule.onAllNodesWithText("Set up").assertCountEquals(0)
    }
}
