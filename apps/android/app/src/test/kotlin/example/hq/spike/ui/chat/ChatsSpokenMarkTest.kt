package example.hq.spike.ui.chat

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
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
class ChatsSpokenMarkTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private fun row(id: String, title: String, subtitle: String, running: Boolean = false) = ChatRow(id, title, subtitle, Group.Today, running, false, spoken = subtitle.startsWith("Spoken"))

    @Test fun aSpokenChatSaysSpokenInItsSubtitle() {
        val state = ChatUiState(chatsOpen = true, chats = listOf(row("a", "Dinner plans", "Spoken · What is for dinner")))
        rule.setContent { HqTheme { ChatsSheet(state, ChatActions()) } }
        rule.onNodeWithText("Dinner plans").assertIsDisplayed()
        rule.onNodeWithText("Spoken · What is for dinner").assertIsDisplayed()
    }

    @Test fun aRunningChatSaysWorkingFromTheCatalogue() {
        val state = ChatUiState(chatsOpen = true, chats = listOf(row("b", "Report", "old", running = true)))
        rule.setContent { HqTheme { ChatsSheet(state, ChatActions()) } }
        rule.onNodeWithText("Working").assertIsDisplayed()
    }
}
