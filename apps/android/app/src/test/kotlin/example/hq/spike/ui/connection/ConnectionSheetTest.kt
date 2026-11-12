package example.hq.spike.ui.connection

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import example.hq.spike.data.Connection
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
@Config(sdk = [35], qualifiers = "w390dp-h1400dp-xxhdpi")
class ConnectionSheetTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private fun show(status: Connection, facts: ConnectionFacts = ConnectionFacts(), actions: ConnectionActions = ConnectionActions(), onClose: () -> Unit = {}) {
        // The step pulse never settles, so time is advanced by hand.
        rule.mainClock.autoAdvance = false
        rule.setContent { HqTheme { ConnectionSheet(status, facts, actions, onClose) } }
        rule.mainClock.advanceTimeBy(200)
    }

    @Test fun aStartShowsTheStepsTheSecondsAndWhatIsUsual() {
        show(Connection.Starting(2, 19, 25L), ConnectionFacts(savedAt = "2:32 PM"))
        rule.onNodeWithText("HQ is starting").assertIsDisplayed()
        rule.onNodeWithText("Step 3 of 4 \u00b7 19 s", substring = true).assertExists()
        rule.onNodeWithContentDescription("Step 3 of 4, Opening your Vault and tasks").assertExists()
        rule.onNodeWithText("Usually about 25 s").assertExists()
        rule.onNodeWithText("2:32 PM").assertExists()
        rule.onAllNodesWithText("Taking longer than usual. HQ is still starting.").assertCountEquals(0)
    }

    @Test fun theFirstStartHasNoUsualTime() {
        show(Connection.Starting(2, 19, null))
        rule.onAllNodesWithText("Usually about", substring = true).assertCountEquals(0)
    }

    @Test fun aSlowButMovingStartSaysSoWithoutNewButtons() {
        show(Connection.Starting(2, 40, 15L))
        rule.onNodeWithText("Taking longer than usual. HQ is still starting.").assertExists()
        rule.onAllNodesWithText("Restart").assertCountEquals(0)
    }

    @Test fun pastAMinuteInStepThreeKeepWaitingClosesAndRestartIsSecond() {
        var closed = 0
        var restarted = 0
        show(Connection.Starting(2, 72, 40L), actions = ConnectionActions(onRestart = { restarted++ }), onClose = { closed++ })
        rule.onNodeWithText("HQ is still opening your Vault. Restarting starts over.").assertExists()
        rule.onNodeWithText("Keep waiting").performScrollTo().performClick()
        assertEquals(1, closed)
        rule.onNodeWithText("Restart").performScrollTo().performClick()
        assertEquals(1, restarted)
    }

    @Test fun underTwoHundredMegabytesTheSheetSaysTheStorageIsAlmostFull() {
        var settings = 0
        show(Connection.Starting(1, 10, null), ConnectionFacts(freeBytes = 80_000_000L), ConnectionActions(onAndroidSettings = { settings++ }))
        rule.onNodeWithText("The phone is almost full. Free up 120 MB and try again.").assertExists()
        rule.onNodeWithText("Android shows what you can remove.").assertExists()
        rule.onNodeWithText("Open Android settings").performScrollTo().performClick()
        assertEquals(1, settings)
    }

    @Test fun aStoppedHqHasTheFactsTheButtonsAndTheTechnicalDetails() {
        val seen = mutableListOf<String>()
        val facts = ConnectionFacts(savedAt = "2:32 PM", stoppedAtStep = 3, details = listOf("Health check timed out after 60 s.", "Last start 9:14 AM \u00b7 exit code none"))
        show(
            Connection.Stopped(0, 2), facts,
            ConnectionActions(onStartHq = { seen += "start" }, onSaveCopy = { seen += "copy" }, onCopyDetails = { seen += "details" }),
        )
        rule.onNodeWithText("HQ has stopped").assertIsDisplayed()
        rule.onNodeWithText("Opening your Vault and tasks").assertExists()
        rule.onNodeWithText("Then set Battery to Unrestricted.").assertExists()
        rule.onNodeWithText("Health check timed out after 60 s.").assertExists()
        rule.onNodeWithText("Start HQ").performScrollTo().performClick()
        rule.onNodeWithText("Save a copy of my Vault").performScrollTo().performClick()
        rule.onNodeWithText("Copy details").performScrollTo().performClick()
        assertEquals(listOf("start", "copy", "details"), seen)
    }

    @Test fun aServerOwnerSeesTheServerRowFirst() {
        show(Connection.Starting(1, 5, null), ConnectionFacts(serverAddress = "hq.example.ts.net"))
        rule.onNodeWithText("hq.example.ts.net \u00b7 Connected").assertExists()
    }
}
