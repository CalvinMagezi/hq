package example.hq.spike.ui.firstrun

import android.app.NotificationManager
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import example.hq.spike.LocalNotif
import example.hq.spike.NotifFactsNow
import example.hq.spike.NotifOrigin
import example.hq.spike.NotifUi
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
@Config(sdk = [35], qualifiers = "w360dp-h640dp-xxhdpi")
class NotifyStepTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private fun facts(sdk: Int = 34, granted: Boolean = false, asked: Boolean = false, rationale: Boolean = false, app: Boolean = true, needs: Int = 4) =
        NotifFactsNow(sdk, granted, app, needs, rationale, asked)

    private fun show(ui: FirstRunUi, f: NotifFactsNow, actions: FirstRunActions = TestActions(), turnOn: (NotifOrigin) -> Unit = {}) {
        rule.setContent {
            HqTheme {
                CompositionLocalProvider(LocalNotif provides NotifUi(f, turnOn)) {
                    androidx.compose.foundation.layout.Column(Modifier.fillMaxSize().verticalScroll(androidx.compose.foundation.rememberScrollState())) {
                        NotifyStep(ui, actions)
                    }
                }
            }
        }
    }

    @Test fun theStepAsksWithAnExampleAndAllowAskingAndroid() {
        val asked = mutableListOf<NotifOrigin>()
        show(FirstRunUi(step = Step.Notify), facts(), turnOn = { asked += it })
        rule.onNodeWithText("Let HQ reach you").assertIsDisplayed()
        rule.onNodeWithText("Allow notifications").performScrollTo().performClick()
        assertEquals(listOf(NotifOrigin.FirstRun), asked)
    }

    @Test fun theExampleIsReadOnceAsOneDescription() {
        show(FirstRunUi(step = Step.Notify), facts())
        rule.onNodeWithContentDescription("Example notification").assertExists()
        assertEquals(0, rule.onAllNodesWithText("Claude needs your OK").fetchSemanticsNodes().size)
    }

    @Test fun passingShowsTheOffFrameWithContinueAndAskAgain() {
        var next = 0
        show(FirstRunUi(step = Step.Notify, notifOff = true), facts(rationale = true, asked = true), TestActions(onNotifNext = { next++ }))
        rule.onNodeWithText("Notifications are off").assertIsDisplayed()
        rule.onNodeWithText("A helper that stops to ask waits until you open the app.").assertExists()
        rule.onNodeWithText("Ask again").performScrollTo().assertIsDisplayed()
        rule.onNodeWithText("Continue").performScrollTo().performClick()
        assertEquals(1, next)
    }

    @Test fun androidThatWillNotAskAgainOffersItsSettings() {
        show(FirstRunUi(step = Step.Notify, notifOff = true), facts(asked = true, rationale = false))
        rule.onNodeWithText("Android will not ask again. Open Android settings and turn on notifications for AgentHQ.").assertExists()
        rule.onNodeWithText("Open Android settings").performScrollTo().assertIsDisplayed()
    }

    @Test fun androidTwelveAndOlderOffersAndroidSettingsOnly() {
        show(FirstRunUi(step = Step.Notify), facts(sdk = 31, granted = true, app = false))
        rule.onNodeWithText("Open Android settings").performScrollTo().assertIsDisplayed()
        assertEquals(0, rule.onAllNodesWithText("Allow notifications").fetchSemanticsNodes().size)
    }

    @Test fun readyToStartShowsAnOffCardWithTurnOnFirst() {
        rule.setContent { HqTheme { CompositionLocalProvider(LocalNotif provides NotifUi(facts(), {})) { StartNotifCard() } } }
        rule.onNodeWithText("Questions will wait in the Inbox.").assertIsDisplayed()
        rule.onNodeWithText("Turn on").assertIsDisplayed()
    }

    @Test fun readyToStartShowsNoCardWhenNotificationsReachTheOwner() {
        rule.setContent { HqTheme { CompositionLocalProvider(LocalNotif provides NotifUi(facts(granted = true), {})) { StartNotifCard() } } }
        assertEquals(0, rule.onAllNodesWithText("Turn on").fetchSemanticsNodes().size)
    }

    @Test fun theMessagingAppsStepIsRenamed() {
        assertEquals("Messaging apps (optional)", FirstRunWords.CHAT_TITLE)
    }
}

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h640dp-xxhdpi")
class NotifyStepLargeTextTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun everyButtonStaysReachableAtLargeTextOn360Dp() {
        rule.setContent {
            HqTheme {
                CompositionLocalProvider(
                    LocalNotif provides NotifUi(NotifFactsNow(34, false, true, NotificationManager.IMPORTANCE_HIGH, false, false), {}),
                    androidx.compose.ui.platform.LocalDensity provides androidx.compose.ui.unit.Density(androidx.compose.ui.platform.LocalDensity.current.density, 2f),
                ) {
                    androidx.compose.foundation.layout.Column(Modifier.fillMaxSize().verticalScroll(androidx.compose.foundation.rememberScrollState())) {
                        NotifyStep(FirstRunUi(step = Step.Notify), TestActions())
                    }
                }
            }
        }
        rule.onNodeWithText("Allow notifications").performScrollTo().assertIsDisplayed()
        rule.onNodeWithText("Not now").performScrollTo().assertIsDisplayed()
        rule.onNodeWithText("Back").performScrollTo().assertIsDisplayed()
    }
}

private fun TestActions(onNotifNext: () -> Unit = {}) = FirstRunActions(
    onNewHq = {}, onRestore = {}, onBack = {}, onProvider = {}, onKey = {}, onShowKey = {}, onBaseUrl = {}, onModelName = {}, onCheck = {},
    onModelNext = {}, onTelegram = {}, onDiscord = {}, onChatNext = {}, onChatSkip = {}, onStart = {}, onBackup = { _, _ -> }, onUseKeyFile = {},
    onPassphrase = {}, onKeyFile = { _, _ -> }, onRunRestore = {}, onContinue = {}, onNotifNext = onNotifNext,
)
