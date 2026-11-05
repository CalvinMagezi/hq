package example.hq.spike.ui.settings

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import example.hq.spike.ui.shell.ConfirmDialog
import example.hq.spike.ui.shell.ConfirmCopy
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
class SpeechPageTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun theFactsAreTheOnesThePhoneKnowsAndDeleteAsksFirst() {
        val asked = mutableListOf<Boolean>()
        rule.setContent { HqTheme { SpeechPage(SettingsUi(speechBytes = 120_000_000L), SettingsActions(onAskDeleteSpeech = { asked += it })) } }
        rule.onNodeWithText("Understands English").assertExists()
        rule.onNodeWithText("The sherpa-onnx project on GitHub").assertExists()
        rule.onNodeWithText("Delete speech model").performClick()
        assertEquals(listOf(true), asked)
    }

    @Test fun deleteNamesTheSizeAndKeepLeavesEverythingAsItWas() {
        var kept = 0
        var deleted = 0
        rule.setContent {
            HqTheme {
                val c = androidx.compose.ui.platform.LocalContext.current
                ConfirmDialog(ConfirmCopy.deleteSpeech(c), { kept++ }, { deleted++ })
            }
        }
        rule.onNodeWithText("Delete the speech model?").assertExists()
        rule.onAllNodesWithText("HQ will need to download", substring = true).assertCountEquals(1)
        rule.onNodeWithText("Keep the speech model").performClick()
        assertEquals(1, kept)
        assertEquals(0, deleted)
    }
}
