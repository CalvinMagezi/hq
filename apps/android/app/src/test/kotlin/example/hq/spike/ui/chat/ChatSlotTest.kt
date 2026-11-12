package example.hq.spike.ui.chat

import example.hq.spike.data.Offer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatSlotTest {
    private val offer = Offer("dated_todo", "Want me to add this to your tasks?", "Add it.", send = true)
    private fun reply(offer: Offer?) = UiMessage("r1", Speaker.Reply, "Done.", offer = offer)

    @Test fun anApprovalWinsOverAnOfferAndNeverBothShow() {
        assertEquals(SlotItem.Approval, ChatSlot.pick(hasApproval = true, offered = offer, result = null))
        assertEquals(SlotItem.Approval, ChatSlot.pick(hasApproval = true, offered = null, result = OfferResult.Later))
        assertEquals(SlotItem.Offer, ChatSlot.pick(hasApproval = false, offered = offer, result = null))
        assertEquals(SlotItem.Offer, ChatSlot.pick(hasApproval = false, offered = null, result = OfferResult.NotNow))
        assertEquals(SlotItem.None, ChatSlot.pick(hasApproval = false, offered = null, result = null))
    }

    @Test fun yesNotNowLaterAreOfferedAndLaterHidesOnceUsedOrUnsupported() {
        assertEquals(listOf(OfferAnswer.Yes, OfferAnswer.NotNow, OfferAnswer.Later), ChatSlot.answers(offer, laterSupported = true))
        assertEquals(listOf(OfferAnswer.Yes, OfferAnswer.NotNow), ChatSlot.answers(offer.copy(laterUsed = true), laterSupported = true))
        assertEquals(listOf(OfferAnswer.Yes, OfferAnswer.NotNow), ChatSlot.answers(offer, laterSupported = false))
    }

    @Test fun theAnswersGoOverTheWireAsHqNamesThem() {
        assertEquals(listOf("yes", "not_now", "later"), OfferAnswer.entries.map { it.wire })
    }

    @Test fun scheduleOnlyStaysForAnOldOfferOnAnHqWithoutLater() {
        val old = offer.copy(kind = "next_step_x")
        assertTrue(ChatSlot.schedulable(old, laterSupported = false))
        assertFalse(ChatSlot.schedulable(old, laterSupported = true))
        assertFalse(ChatSlot.schedulable(offer, laterSupported = false))
    }

    @Test fun laterUsedIsReadFromTheSavedOffer() {
        val json = org.json.JSONObject("""{"kind":"k","line":"l","reply":"r","send":false,"later_used":true}""")
        assertTrue(Offer.from(json)!!.laterUsed)
        assertFalse(Offer.from(org.json.JSONObject("""{"kind":"k","line":"l","reply":"r"}"""))!!.laterUsed)
    }

    @Test fun onlyTheNewestFinishedReplyOffersAndNotWhileOneIsWrittenOrFailed() {
        val s = ChatUiState(messages = listOf(UiMessage("u", Speaker.User, "hi"), reply(offer)))
        assertEquals("r1", s.offered?.id)
        assertNull(s.copy(live = LiveReply()).offered)
        assertNull(s.copy(failed = true).offered)
        assertNull(s.copy(messages = s.messages + UiMessage("u2", Speaker.User, "more")).offered)
        assertNull(s.copy(messages = listOf(reply(null))).offered)
    }

    @Test fun theRightButtonIsVoiceWhenEmptySendWithTextAndStopWhileWorking() {
        assertEquals(SendKey.Voice, ChatSlot.sendKey(running = false, draft = "", hasAttachments = false))
        assertEquals(SendKey.Voice, ChatSlot.sendKey(running = false, draft = "  \n", hasAttachments = false))
        assertEquals(SendKey.Send, ChatSlot.sendKey(running = false, draft = "Remind me", hasAttachments = false))
        assertEquals(SendKey.Send, ChatSlot.sendKey(running = false, draft = "", hasAttachments = true))
        assertEquals(SendKey.Stop, ChatSlot.sendKey(running = true, draft = "", hasAttachments = false))
        assertEquals(SendKey.Stop, ChatSlot.sendKey(running = true, draft = "typing", hasAttachments = true))
    }

    @Test fun buttonsStackFromFontScaleOnePointThree() {
        assertFalse(SlotLayout.stacked(1.2f))
        assertTrue(SlotLayout.stacked(1.3f))
        assertTrue(SlotLayout.stacked(2.0f))
    }

    @Test fun onlyAbsentRoutesHideFeaturesAndA404MeansNoOpenOffer() {
        assertEquals(OfferFailure.Kind.RouteMissing, OfferFailure.classify(405))
        assertEquals(OfferFailure.Kind.RouteMissing, OfferFailure.classify(501))
        assertEquals(OfferFailure.Kind.NoOpenOffer, OfferFailure.classify(404))
        assertEquals(OfferFailure.Kind.BadRequest, OfferFailure.classify(400))
        assertEquals(OfferFailure.Kind.LaterUsed, OfferFailure.classify(409))
        assertEquals(OfferFailure.Kind.Transient, OfferFailure.classify(0))
        assertEquals(OfferFailure.Kind.Transient, OfferFailure.classify(503))
    }

    private fun row(id: String) = ChatRow(id, id, "", Group.Today, running = false, unread = false)

    @Test fun pinsStartWithTheNewestFirstAndStopAtSix() {
        var ids = emptyList<String>()
        for (i in 1..Pins.MAX) ids = (Pins.toggle(ids, "c$i") as Pins.Change.Done).ids
        assertEquals(listOf("c6", "c5", "c4", "c3", "c2", "c1"), ids)
        assertEquals(Pins.Change.Full, Pins.toggle(ids, "c7"))
        val unpinned = Pins.toggle(ids, "c3") as Pins.Change.Done
        assertFalse(unpinned.pinned)
        assertEquals(listOf("c6", "c5", "c4", "c2", "c1"), unpinned.ids)
    }

    @Test fun aPinOlderThanTheFirstPageOfChatsIsNeverPrunedByAToggle() {
        // HQ lists 50 chats; "old" is pinned but not in that page. Pinning another must keep it.
        val done = Pins.toggle(listOf("old"), "new") as Pins.Change.Done
        assertEquals(listOf("new", "old"), done.ids)
    }

    @Test fun onlyAChatHqSaysIsGoneLosesItsPin() {
        assertEquals(listOf("a"), Pins.forget(listOf("a", "b"), "b"))
        assertEquals(listOf("a", "b"), Pins.forget(listOf("a", "b"), "zzz"))
    }

    @Test fun aPinnedRowHiddenDuringTheArchiveWindowKeepsItsPinForUndo() {
        val ids = listOf("a", "b")
        // The sheet drops the archived row from the list; the stored ids are untouched until the window closes.
        val shown = Pins.apply(listOf(row("b"), row("c")), ids)
        assertEquals(listOf("b", "c"), shown.map { it.id })
        assertEquals(ids, Pins.parse(Pins.encode(ids)))
        assertEquals(listOf("a", "b"), Pins.apply(listOf(row("a"), row("b")), ids).map { it.id })
    }

    @Test fun pinnedRowsComeFirstInPinOrderAndTheRestKeepTheirOrder() {
        val rows = listOf(row("a"), row("b"), row("c"), row("d"))
        val out = Pins.apply(rows, listOf("c", "a"))
        assertEquals(listOf("c", "a", "b", "d"), out.map { it.id })
        assertEquals(listOf(true, true, false, false), out.map { it.pinned })
    }

    @Test fun pinsRoundTripThroughThePreferencesString() {
        assertEquals(listOf("a", "b"), Pins.parse(Pins.encode(listOf("a", "b"))))
        assertEquals(emptyList<String>(), Pins.parse(null))
        assertEquals(listOf("a"), Pins.parse(",a,,a"))
    }
}
