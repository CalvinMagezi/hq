package example.hq.spike.voice

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import example.hq.spike.download.DlState
import example.hq.spike.download.Item
import example.hq.spike.measure.DataPurpose
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
class VoiceSheetTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
    private val item = Item("speech", "Speech model", "https://example.com/m.tar.bz2", 57_000_000L, "0".repeat(64), DataPurpose.SpeechModel, 120_000_000L, "m.tar.bz2", speech = true)

    private fun show(phase: VoicePhase, dl: DlState = DlState.Ready, onMic: () -> Unit = {}, onClose: () -> Unit = {}) {
        rule.setContent { HqTheme { VoiceSheet(VoiceState(phase), dl, item, {}, { null }, onMic, onClose) } }
    }

    @Test fun withoutAModelTheDownloadRowAndNotNowTakeThePlaceOfTheMic() {
        var closed = 0
        show(VoicePhase.NeedsModel, onClose = { closed++ })
        rule.onNodeWithText("Hold Home to talk to HQ").assertIsDisplayed()
        rule.onAllNodesWithText("Ask").assertCountEquals(0)
        rule.onNodeWithText("Not now").performClick()
        assertEquals(1, closed)
    }

    @Test fun whileItDownloadsThereIsNoNotNowAndNoMic() {
        show(VoicePhase.Downloading, DlState.Downloading(1_000_000L, onPaid = false))
        rule.onNodeWithText("Getting the speech model").assertIsDisplayed()
        rule.onAllNodesWithText("Ask").assertCountEquals(0)
    }

    @Test fun onceReadyItOffersAskAndNeverStartsListeningByItself() {
        var taps = 0
        show(VoicePhase.ModelReady, onMic = { taps++ })
        rule.onNodeWithText("The speech model is ready").assertIsDisplayed()
        assertEquals(0, taps)
        rule.onNodeWithText("Ask").performClick()
        assertEquals(1, taps)
    }

    @Test fun aFirstRefusalShowsAllowAndTypeInstead() {
        var typed = 0
        rule.setContent { HqTheme { VoiceSheet(VoiceState(VoicePhase.MicNeeded), DlState.Ready, item, {}, { null }, {}, {}, onType = { typed++ }) } }
        rule.onNodeWithText("Microphone needed").assertIsDisplayed()
        rule.onNodeWithText("Type instead").performClick()
        assertEquals(1, typed)
    }

    @Test fun aBlockedMicrophoneOpensAndroidSettings() {
        var opened = 0
        rule.setContent { HqTheme { VoiceSheet(VoiceState(VoicePhase.MicBlocked), DlState.Ready, item, {}, { null }, {}, {}, onSettings = { opened++ }) } }
        rule.onNodeWithText("Microphone off").assertIsDisplayed()
        rule.onNodeWithText("Ask again").assertIsDisplayed()
        rule.onNodeWithText("Open Android settings").performClick()
        assertEquals(1, opened)
    }

    @Test fun openInChatShowsOnceTheThreadIsKnown() {
        var id: String? = null
        rule.setContent { HqTheme { VoiceSheet(VoiceState(VoicePhase.Answering, reply = "Hi", threadId = "t1"), DlState.Ready, item, {}, { null }, {}, {}, onOpenChat = { id = it }) } }
        rule.onNodeWithText("Open in chat").performClick()
        assertEquals("t1", id)
    }
}
