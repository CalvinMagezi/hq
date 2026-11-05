package example.hq.spike.ui.inbox

import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
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
@Config(sdk = [35], qualifiers = "w390dp-h1200dp-xxhdpi")
class InboxBannerTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val off = NotifFactsNow(34, false, true, 4, false, false)

    private fun card(title: String, body: String, label: String = "Allow") =
        InboxCard("n1", CardKind.Approval, title, body, "", "2025-01-01T00:00:00Z", CardIcon.Info, null, true, label, null, "", "pending", loses = title.startsWith("Delete"), verb = "delete")

    private fun show(ui: InboxUi, actions: InboxActions = InboxActions(), turnOn: (NotifOrigin) -> Unit = {}) {
        rule.setContent { HqTheme { CompositionLocalProvider(LocalNotif provides NotifUi(off, turnOn)) { InboxScreen(ui, actions, onTakeLook = {}) } } }
    }

    @Test fun theBannerSaysHowManyWaitedAndTurnOnAsksAndroid() {
        val turned = mutableListOf<NotifOrigin>()
        show(InboxUi(loaded = true, needs = listOf(card("Run tests?", "ok")), banner = InboxBanner(2, 0)), turnOn = { turned += it })
        rule.onNodeWithText("2 questions waited here without telling you.").assertIsDisplayed()
        rule.onNodeWithText("Turn on").performClick()
        assertEquals(listOf(NotifOrigin.Later), turned)
    }

    @Test fun oneQuestionReadsInTheSingular() {
        show(InboxUi(loaded = true, needs = listOf(card("Run tests?", "ok")), banner = InboxBanner(1, 0)))
        rule.onNodeWithText("1 question waited here without telling you.").assertIsDisplayed()
    }

    @Test fun theNoneYetTextAndTheServerLineKeepTheirOwnCount() {
        show(InboxUi(loaded = true, needs = listOf(card("Run tests?", "ok")), banner = InboxBanner(0, 3)))
        rule.onNodeWithText("Questions wait here without telling you.").assertIsDisplayed()
        rule.onNodeWithText("3 are waiting on your server HQ.").assertIsDisplayed()
    }

    @Test fun notNowOnTheBannerIsReported() {
        var snoozed = 0
        show(InboxUi(loaded = true, needs = listOf(card("Run tests?", "ok")), banner = InboxBanner(1, 0)), InboxActions(onBannerNotNow = { snoozed++ }))
        rule.onNodeWithText("Not now").performClick()
        assertEquals(1, snoozed)
    }

    @Test fun noBannerWhenNotificationsAreOn() {
        show(InboxUi(loaded = true, needs = listOf(card("Run tests?", "ok"))))
        assertEquals(0, rule.onAllNodesWithText("Turn on").fetchSemanticsNodes().size)
    }

    @Test fun declineSitsLeftOfAllowOnceAndNoNotNowRemains() {
        show(InboxUi(loaded = true, needs = listOf(card("Run tests?", "ok"))))
        val decline = rule.onNodeWithText("Decline").fetchSemanticsNode().boundsInRoot
        val allow = rule.onNodeWithText("Allow once").fetchSemanticsNode().boundsInRoot
        assert(decline.right <= allow.left)
        assertEquals(0, rule.onAllNodesWithText("Not now").fetchSemanticsNodes().size)
    }

    @Test fun aRequestThatDeletesShowsARedAllowDeleteOnTheRight() {
        show(InboxUi(loaded = true, needs = listOf(card("Delete 3 old backups?", "The task wants to delete the oldest copies."))))
        val decline = rule.onNodeWithText("Decline").fetchSemanticsNode().boundsInRoot
        val allow = rule.onNodeWithText("Allow delete").fetchSemanticsNode().boundsInRoot
        assert(decline.right <= allow.left)
        assertEquals(0, rule.onAllNodesWithText("Allow once").fetchSemanticsNodes().size)
    }
}
