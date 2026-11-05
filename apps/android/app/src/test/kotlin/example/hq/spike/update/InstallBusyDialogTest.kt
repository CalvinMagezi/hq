package example.hq.spike.update

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import example.hq.spike.ui.theme.HqTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** The dialog names what an install would stop, puts the safe choice first, and offers the right stop label. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class InstallBusyDialogTest {
    @get:Rule val rule = createComposeRule()

    @Test fun chatShowsBothChoicesAndWaitIsWired() {
        var waited = 0
        var stopped = 0
        rule.setContent { HqTheme { InstallBusyDialog(BusyReason.Chat, { waited++ }, { stopped++ }, {}) } }
        rule.onNodeWithText("HQ is answering a chat.").assertIsDisplayed()
        rule.onNodeWithText("Stop the chat and install").assertIsDisplayed()
        rule.onNodeWithText("Wait until it's done").performClick()
        assertEquals(1, waited)
        rule.onNodeWithText("Stop the chat and install").performClick()
        assertEquals(1, stopped)
    }

    @Test fun otherReasonsSayStopItAndInstall() {
        rule.setContent { HqTheme { InstallBusyDialog(BusyReason.Backup, {}, {}, {}) } }
        rule.onNodeWithText("Stop it and install").assertIsDisplayed()
    }
}
