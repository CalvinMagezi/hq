package example.hq.spike.ui.chat

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import example.hq.spike.data.ChatEvent
import example.hq.spike.data.Offer
import example.hq.spike.data.ToolStep
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class LaterAsksTest {
    private val context: Context get() = ApplicationProvider.getApplicationContext()

    @Test fun calendarAskTriggersOnlyOnCalendarDeviceActionAndChecksOutcome() {
        assertTrue(CalendarAsk.shouldShow(isCalendarRequest = true, hasPermission = false, askedBefore = false))
        assertFalse(CalendarAsk.shouldShow(isCalendarRequest = false, hasPermission = false, askedBefore = false))
        assertFalse(CalendarAsk.shouldShow(isCalendarRequest = true, hasPermission = true, askedBefore = false))
        assertFalse(CalendarAsk.shouldShow(isCalendarRequest = true, hasPermission = false, askedBefore = true))

        assertTrue(CalendarAsk.isCalendarEvent(ChatEvent.DeviceAction("1", "t1", "calendar.create", 1, JSONObject(), null)))
        assertFalse(CalendarAsk.isCalendarEvent(ChatEvent.ToolStarted("t1", ToolStep("id", "calendar_list", null, null, null))))
        assertFalse(CalendarAsk.isCalendarEvent(ChatEvent.ToolStarted("t1", ToolStep("id", "note_read", null, null, null))))

        assertEquals(
            CalendarPermOutcome.Granted,
            CalendarAsk.permissionOutcome(readGranted = true, writeGranted = true, rationaleRead = false, rationaleWrite = false),
        )
        assertEquals(
            CalendarPermOutcome.Denied,
            CalendarAsk.permissionOutcome(readGranted = false, writeGranted = false, rationaleRead = true, rationaleWrite = false),
        )
        assertEquals(
            CalendarPermOutcome.PermanentlyDenied,
            CalendarAsk.permissionOutcome(readGranted = false, writeGranted = false, rationaleRead = false, rationaleWrite = false),
        )
    }

    @Test fun messagingOfferShowsAfterThreeChatsWithoutRelayToken() {
        assertFalse(MessagingOffer.shouldShow(chatCount = 2, hasRelayToken = false, askedBefore = false, replyDone = true))
        assertTrue(MessagingOffer.shouldShow(chatCount = 3, hasRelayToken = false, askedBefore = false, replyDone = true))
        assertFalse(MessagingOffer.shouldShow(chatCount = 3, hasRelayToken = true, askedBefore = false, replyDone = true))
        assertFalse(MessagingOffer.shouldShow(chatCount = 3, hasRelayToken = false, askedBefore = true, replyDone = true))
        assertFalse(MessagingOffer.shouldShow(chatCount = 3, hasRelayToken = false, askedBefore = false, replyDone = false))
    }

    @Test fun assistantOfferShowsAfterFirstVoiceTurnWhenAvailableAndNotHeld() {
        assertTrue(AssistantOffer.shouldShow(voiceTurnDone = true, available = true, held = false, askedBefore = false))
        assertFalse(AssistantOffer.shouldShow(voiceTurnDone = false, available = true, held = false, askedBefore = false))
        assertFalse(AssistantOffer.shouldShow(voiceTurnDone = true, available = false, held = false, askedBefore = false))
        assertFalse(AssistantOffer.shouldShow(voiceTurnDone = true, available = true, held = true, askedBefore = false))
        assertFalse(AssistantOffer.shouldShow(voiceTurnDone = true, available = true, held = false, askedBefore = true))
    }

    @Test fun repoOfferMatchesOnlyPlainGithubRepoUrlsAndRejectsFalsePositives() {
        assertTrue(RepoOffer.containsRepoUrl("Check https://github.com/example/hq-oss please"))
        assertTrue(RepoOffer.containsRepoUrl("See www.github.com/example/hq-oss"))
        assertTrue(RepoOffer.containsRepoUrl("See github.com/owner/repo-name for details"))

        assertFalse(RepoOffer.containsRepoUrl("Check https://gitlab.com/example/hq-oss"))
        assertFalse(RepoOffer.containsRepoUrl("Check https://bitbucket.org/example/hq-oss"))
        assertFalse(RepoOffer.containsRepoUrl("Check https://codeberg.org/example/hq-oss"))
        assertFalse(RepoOffer.containsRepoUrl("Check notgithub.com/example/hq-oss"))
        assertFalse(RepoOffer.containsRepoUrl("Open https://github.com/settings/profile"))
        assertFalse(RepoOffer.containsRepoUrl("Open https://github.com/orgs/example"))
        assertFalse(RepoOffer.containsRepoUrl("Inline `github.com/example/hq-oss` code"))
        assertFalse(RepoOffer.containsRepoUrl("```\ngithub.com/example/hq-oss\n```"))
        assertFalse(RepoOffer.containsRepoUrl("> https://github.com/example/hq-oss"))

        assertTrue(RepoOffer.shouldShow("Fix https://github.com/example/hq-oss", githubConnected = false, askedBefore = false))
        assertFalse(RepoOffer.shouldShow("Fix https://github.com/example/hq-oss", githubConnected = true, askedBefore = false))
        assertFalse(RepoOffer.shouldShow("Fix https://github.com/example/hq-oss", githubConnected = false, askedBefore = true))
    }

    @Test fun laterAsksPrefsRemembersEachKindAndVoiceTurn() {
        for (kind in LocalOfferKind.entries) {
            assertFalse(LaterAsksPrefs.wasAsked(context, kind))
            LaterAsksPrefs.markAsked(context, kind)
            assertTrue(LaterAsksPrefs.wasAsked(context, kind))
        }
        assertFalse(LaterAsksPrefs.voiceTurnDone(context))
        LaterAsksPrefs.markVoiceTurnDone(context)
        assertTrue(LaterAsksPrefs.voiceTurnDone(context))
    }

    @Test fun aLocalOfferOutranksFollowUpChipsAndNeverTheOtherWayAround() {
        val local = LocalOffer(LocalOfferKind.Repo, "Connect GitHub?", "Yes", "Not now")
        assertEquals(SlotItem.LocalOffer, ChatSlot.pick(false, null, null, hasChips = true, localOffer = local))
        assertEquals(SlotItem.Chips, ChatSlot.pick(false, null, null, hasChips = true, localOffer = null))
        assertEquals(SlotItem.Approval, ChatSlot.pick(true, null, null, hasChips = true, localOffer = local))
    }

    @Test fun chatSlotRanksApprovalOverServerOfferOverLocalOffer() {
        val local = LocalOffer(LocalOfferKind.Repo, "Connect GitHub?", "Yes", "Not now")
        val server = Offer("kind", "Server offer", "reply", send = false)
        assertEquals(SlotItem.Approval, ChatSlot.pick(hasApproval = true, offered = server, result = null, localOffer = local))
        assertEquals(SlotItem.Offer, ChatSlot.pick(hasApproval = false, offered = server, result = null, localOffer = local))
        assertEquals(SlotItem.LocalOffer, ChatSlot.pick(hasApproval = false, offered = null, result = null, localOffer = local))
        assertEquals(SlotItem.None, ChatSlot.pick(hasApproval = false, offered = null, result = null, localOffer = null))
    }

    @Test fun laterAskDeciderAndSessionFlagDriveOffersCleanly() {
        val local = LocalOffer(LocalOfferKind.Repo, "Connect GitHub?", "Yes", "Not now")
        val server = Offer("kind", "Server offer", "reply", send = false)
        assertFalse(LaterAskDecider.blockedByOtherSlot(connected = true, hasChatNeeds = false, offered = null, offerResult = null, localOffer = null))
        assertTrue(LaterAskDecider.blockedByOtherSlot(connected = false, hasChatNeeds = false, offered = null, offerResult = null, localOffer = null))
        assertTrue(LaterAskDecider.blockedByOtherSlot(connected = true, hasChatNeeds = true, offered = null, offerResult = null, localOffer = null))
        assertTrue(LaterAskDecider.blockedByOtherSlot(connected = true, hasChatNeeds = false, offered = server, offerResult = null, localOffer = null))
        assertTrue(LaterAskDecider.blockedByOtherSlot(connected = true, hasChatNeeds = false, offered = null, offerResult = OfferResult.NotNow, localOffer = null))
        assertTrue(LaterAskDecider.blockedByOtherSlot(connected = true, hasChatNeeds = false, offered = null, offerResult = null, localOffer = local))

        val chats = listOf(
            ChatRow("1", "One", "", Group.Today, running = false, unread = false),
            ChatRow("2", "Two", "", Group.Today, running = false, unread = false),
            ChatRow("3", "Three", "", Group.Today, running = false, unread = false),
        )
        val initial = ChatUiState(threadId = "1", chats = chats)
        assertFalse(initial.replyCompletedInSession)
        val started = ChatReducer.apply(initial, ChatEvent.TurnStart("1", "c1", null), setOf("c1"))
        val streamed = ChatReducer.apply(started, ChatEvent.Text("1", "Hello"), emptySet())
        val ended = ChatReducer.apply(streamed, ChatEvent.TurnEnd("1", "m1", stopped = false), emptySet())
        assertTrue(ended.replyCompletedInSession)
        assertTrue(LaterAskDecider.shouldOfferMessagingOnTurnEnd(ended, hasRelayToken = false, askedBefore = false, threadMatches = true, stopped = false))
        assertFalse(LaterAskDecider.shouldOfferMessagingOnTurnEnd(ended, hasRelayToken = false, askedBefore = false, threadMatches = true, stopped = true))
        val nextStarted = ChatReducer.apply(ended, ChatEvent.TurnStart("1", "c2", null), setOf("c2"))
        assertFalse(nextStarted.replyCompletedInSession)

        assertTrue(LaterAskDecider.shouldOfferRepoOnSubmit(fromComposer = true, sendSucceeded = true, text = "github.com/a/b", githubConnected = false, askedBefore = false, hasLocalOffer = false))
        assertFalse(LaterAskDecider.shouldOfferRepoOnSubmit(fromComposer = false, sendSucceeded = true, text = "github.com/a/b", githubConnected = false, askedBefore = false, hasLocalOffer = false))
        assertFalse(LaterAskDecider.shouldOfferRepoOnSubmit(fromComposer = true, sendSucceeded = false, text = "github.com/a/b", githubConnected = false, askedBefore = false, hasLocalOffer = false))
        assertTrue(LaterAskDecider.shouldOfferAssistantOnResume(hasLocalOffer = false, voiceTurnDone = true, available = true, held = false, askedBefore = false))
        assertFalse(LaterAskDecider.shouldOfferAssistantOnResume(hasLocalOffer = true, voiceTurnDone = true, available = true, held = false, askedBefore = false))
    }
}
