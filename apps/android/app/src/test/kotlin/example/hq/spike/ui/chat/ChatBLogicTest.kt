package example.hq.spike.ui.chat

import example.hq.spike.data.ChatEvent
import example.hq.spike.data.Connection
import example.hq.spike.data.Offer
import example.hq.spike.data.ToolStep
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatBLogicTest {
    private fun step(name: String) = ToolStep(id = "t1", name = name, args = null, result = null, durationMs = null)
    private fun reply(id: String = "r1", tools: List<ToolStep> = emptyList(), offer: Offer? = null) =
        UiMessage(id, Speaker.Reply, "Done.", tools = tools, offer = offer)

    @Test fun followUpChipsPickByToolKind() {
        assertEquals(ReplyToolKind.NoteSaved, FollowUpChips.classify(listOf(step("vault_write_note"))))
        assertEquals(ReplyToolKind.TaskMade, FollowUpChips.classify(listOf(step("task_create"))))
        assertEquals(ReplyToolKind.SearchOrRead, FollowUpChips.classify(listOf(step("web_search"))))
        assertEquals(ReplyToolKind.None, FollowUpChips.classify(emptyList()))
        assertEquals(FollowUpChips.TABLE[ReplyToolKind.NoteSaved], FollowUpChips.forTools(listOf(step("vault_write_note"))))
    }

    @Test fun followUpChipsRequireTurnEndInThisSessionAndConnected() {
        val oldChat = ChatUiState(
            threadId = "t1",
            connected = true,
            messages = listOf(UiMessage("u1", Speaker.User, "hi"), reply("r1")),
            finishedReplyId = null,
        )
        // Old chat opened with a final reply must not show follow-up chips.
        assertTrue(FollowUpChips.forState(oldChat, hasApproval = false, hasProblemBar = false).isEmpty())

        // After TurnEnd in this session, finishedReplyId is set and chips appear.
        val afterEnd = ChatReducer.apply(
            oldChat.copy(messages = listOf(UiMessage("u1", Speaker.User, "hi")), live = LiveReply(text = "Done.")),
            ChatEvent.TurnEnd(threadId = "t1", messageId = "r1", stopped = false),
            emptySet(),
        )
        assertEquals("r1", afterEnd.finishedReplyId)
        assertEquals(FollowUpChips.TABLE[ReplyToolKind.None], FollowUpChips.forState(afterEnd, hasApproval = false, hasProblemBar = false))

        // Offline must hide chips.
        assertTrue(FollowUpChips.forState(afterEnd.copy(connected = false), hasApproval = false, hasProblemBar = false).isEmpty())
    }

    @Test fun followUpChipsVanishOnTypingAndStayGoneWhenCleared() {
        val base = ChatUiState(
            messages = listOf(UiMessage("u1", Speaker.User, "hi"), reply("r1")),
            finishedReplyId = "r1",
        )
        assertEquals(FollowUpChips.TABLE[ReplyToolKind.None], FollowUpChips.forState(base, hasApproval = false, hasProblemBar = false))

        val typed = FollowUpChips.onDraftChanged(base, "h")
        assertEquals("r1", typed.dismissedFollowUpId)
        assertTrue(FollowUpChips.forState(typed, hasApproval = false, hasProblemBar = false).isEmpty())

        val cleared = FollowUpChips.onDraftChanged(typed, "")
        assertEquals("r1", cleared.dismissedFollowUpId)
        assertTrue(FollowUpChips.forState(cleared, hasApproval = false, hasProblemBar = false).isEmpty())

        assertTrue(FollowUpChips.forState(base.copy(messages = base.messages + UiMessage("u2", Speaker.User, "next")), hasApproval = false, hasProblemBar = false).isEmpty())
        assertTrue(FollowUpChips.forState(base.copy(live = LiveReply()), hasApproval = false, hasProblemBar = false).isEmpty())
        assertTrue(FollowUpChips.forState(base.copy(failed = true), hasApproval = false, hasProblemBar = false).isEmpty())
        assertTrue(FollowUpChips.forState(base, hasApproval = true, hasProblemBar = false).isEmpty())
        assertTrue(FollowUpChips.forState(base, hasApproval = false, hasProblemBar = true).isEmpty())
        val withOffer = base.copy(messages = listOf(reply("r2", offer = Offer("k", "line", "yes", true))), finishedReplyId = "r2")
        assertTrue(FollowUpChips.forState(withOffer, hasApproval = false, hasProblemBar = false).isEmpty())
    }

    @Test fun problemBarShowsOnlyForOfflineAndSignedOut() {
        assertEquals(ProblemBar.State.Offline, ProblemBar.forState(Connection.Offline(0)))
        assertEquals(ProblemBar.State.SignedOut, ProblemBar.forState(Connection.ServerSignedOut))
        assertNull(ProblemBar.forState(Connection.Connected))
        assertNull(ProblemBar.forState(Connection.Reconnecting(0)))
    }

    @Test fun failedEarlierMessageKeepsNotSentAfterLaterSendWhileSendingIsLastOnly() {
        val earlierFailed = UiMessage("local-1", Speaker.User, "first", notSent = true)
        val earlierSending = UiMessage("local-1", Speaker.User, "first", sending = true)
        val latest = UiMessage("local-2", Speaker.User, "second", sending = true)

        val withFailedFirst = listOf(earlierFailed, reply("r1"), latest)
        assertEquals(SendStatus.NotSent, MessageSendState.statusFor(withFailedFirst, "local-1"))
        assertEquals(SendStatus.Sending, MessageSendState.statusFor(withFailedFirst, "local-2"))

        val withSendingFirst = listOf(earlierSending, reply("r1"), latest)
        assertNull(MessageSendState.statusFor(withSendingFirst, "local-1"))
        assertEquals(SendStatus.Sending, MessageSendState.statusFor(withSendingFirst, "local-2"))
    }

    @Test fun socketDropFlipsInFlightToNotSentAndReconnectDeduplicatesByClientId() {
        val sending = UiMessage("local-1", Speaker.User, "hello", sending = true)
        val flipped = MessageSendState.failInFlight(listOf(sending))
        assertFalse(flipped.first().sending)
        assertTrue(flipped.first().notSent)

        val savedFromServer = listOf(UiMessage("srv-99", Speaker.User, "hello", clientId = "local-1"))
        val stillUnsaved = MessageSendState.unsavedLocals(flipped, savedFromServer)
        assertTrue(stillUnsaved.isEmpty())

        val otherUnsent = UiMessage("local-2", Speaker.User, "still waiting", notSent = true)
        assertEquals(listOf("local-2"), MessageSendState.unsavedLocals(flipped + otherUnsent, savedFromServer).map { it.id })
    }

    @Test fun beginRetryTransitionsOnceAndBlocksSecondTap() {
        val failed = UiMessage("local-1", Speaker.User, "hello", notSent = true)
        val (firstMsgs, firstTarget) = MessageSendState.beginRetry(listOf(failed), "local-1")
        assertNotNull(firstTarget)
        assertTrue(firstMsgs.first().sending)
        assertFalse(firstMsgs.first().notSent)

        val (_, secondTarget) = MessageSendState.beginRetry(firstMsgs, "local-1")
        assertNull(secondTarget)
    }

    @Test fun shouldReloadOnRejectOnlyWhenTailWasReplaced() {
        assertTrue(MessageSendState.shouldReloadOnReject(replacedId = "u1", threadId = "t1"))
        assertFalse(MessageSendState.shouldReloadOnReject(replacedId = null, threadId = "t1"))
        assertFalse(MessageSendState.shouldReloadOnReject(replacedId = "u1", threadId = null))
    }

    @Test fun hapticsTickOnlyWhenTouchFeedbackOnAndAnimationsEnabled() {
        assertTrue(ChatHaptics.shouldTick(touchFeedbackEnabled = true, animatorDurationScale = 1.0f))
        assertFalse(ChatHaptics.shouldTick(touchFeedbackEnabled = false, animatorDurationScale = 1.0f))
        assertFalse(ChatHaptics.shouldTick(touchFeedbackEnabled = true, animatorDurationScale = 0.0f))
        assertFalse(ChatHaptics.shouldTick(touchFeedbackEnabled = false, animatorDurationScale = 0.0f))
    }

    @Test fun chatsFilterMatchesTitleOnly() {
        val rows = listOf(
            ChatRow("1", "Weekly review", "subtitle match", Group.Today, running = false, unread = false),
            ChatRow("2", "Grocery list", "Weekly", Group.Today, running = false, unread = false),
        )
        assertEquals(listOf("1"), ChatsFilter.byTitle(rows, "weekly").map { it.id })
        assertEquals(listOf("1", "2"), ChatsFilter.byTitle(rows, "  ").map { it.id })
    }
}
