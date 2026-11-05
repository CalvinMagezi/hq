package example.hq.spike.ui.settings

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onAllNodesWithText
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

    @Test fun appOffShowsTheCardTurnOnAndOffOnEveryChannel() {
        val channels = listOf(ChannelFacts("needs", "Needs you", 4, true), ChannelFacts("updates", "Updates", 3, false))
        show(NotifFacts(allowed = false, channels = channels))
        rule.onNodeWithText("Notifications are off for AgentHQ").assertIsDisplayed()
        rule.onNodeWithText("Approvals wait in the Inbox until they are on again.").assertIsDisplayed()
        rule.onNodeWithText("Turn on").assertIsDisplayed()
        assertEquals(2, rule.onAllNodesWithText("Off in Android settings").fetchSemanticsNodes().size)
    }

    @Test fun needsYouOffShowsTheAmberCardAndTurnOn() {
        show(NotifFacts(needsImportance = 0))
        rule.onNodeWithText("Needs you is off").assertIsDisplayed()
        rule.onNodeWithText("Questions wait in the Inbox without telling you.").assertIsDisplayed()
        rule.onNodeWithText("Turn on").assertIsDisplayed()
    }

    @Test fun aQuieterNeedsYouGetsANoteAndNoCard() {
        show(NotifFacts(needsImportance = 3))
        rule.onNodeWithText("Needs you makes a sound but no longer pops up on screen.").assertIsDisplayed()
        assertEquals(0, rule.onAllNodesWithText("Turn on").fetchSemanticsNodes().size)
    }

    @Test fun aSilentNeedsYouGetsTheQuietNote() {
        show(NotifFacts(needsImportance = 2))
        rule.onNodeWithText("Needs you arrives silently, without a pop-up.").assertIsDisplayed()
    }

    @Test fun theUpgraderCardShowsOnceAndIsMarkedSeen() {
        var seen = 0
        show(NotifFacts(upgraderNote = true), SettingsActions(onUpgraderNoteSeen = { seen++ }))
        rule.onNodeWithText("Notifications now hide details").assertIsDisplayed()
        assertEquals(1, seen)
    }
}
