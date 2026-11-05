package example.hq.spike.ui.settings

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.isToggleable
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
@Config(sdk = [35], qualifiers = "w390dp-h1200dp-xxhdpi")
class NotificationsPageTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private fun show(facts: NotifFacts, actions: SettingsActions = SettingsActions()) {
        rule.setContent { HqTheme { Notifications(SettingsUi(notif = facts), actions) } }
    }

    @Test fun turningDetailsOnAsksFirstAndKeepHiddenLeavesItOff() {
        val seen = mutableListOf<Boolean>()
        show(NotifFacts(), SettingsActions(onDetails = { seen += it }))
        rule.onNode(isToggleable()).performClick()
        rule.onNodeWithText("Show details in notifications?").assertExists()
        rule.onNodeWithText("Keep hidden").performClick()
        assertEquals(emptyList<Boolean>(), seen)
    }

    @Test fun showDetailsInTheDialogTurnsItOn() {
        val seen = mutableListOf<Boolean>()
        show(NotifFacts(), SettingsActions(onDetails = { seen += it }))
        rule.onNode(isToggleable()).performClick()
        rule.onNodeWithText("Show details").performClick()
        assertEquals(listOf(true), seen)
    }

    @Test fun noScreenLockSaysSoAndOffersAndroidSettings() {
        show(NotifFacts(secure = false))
        rule.onNodeWithText("Needs a screen lock").assertIsDisplayed()
        rule.onNodeWithText("Open Android settings").assertIsDisplayed()
    }

    @Test fun theUpgraderCardShowsOnceAndIsMarkedSeen() {
        var seen = 0
        show(NotifFacts(upgraderNote = true), SettingsActions(onUpgraderNoteSeen = { seen++ }))
        rule.onNodeWithText("Notifications now hide details").assertIsDisplayed()
        assertEquals(1, seen)
    }
}
