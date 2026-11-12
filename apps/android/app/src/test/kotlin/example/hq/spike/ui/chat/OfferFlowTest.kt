package example.hq.spike.ui.chat

import example.hq.spike.data.Offer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class OfferFlowTest {
    private val offer = Offer("dated_todo", "Add it to your tasks?", "Add it.", send = true)
    private val t = FlowTexts(retry = "Couldn't send that. Try again.", gone = "That suggestion was already answered.")
    private fun chat() = ChatUiState(messages = listOf(UiMessage("u", Speaker.User, "hi"), UiMessage("r1", Speaker.Reply, "Done.", offer = offer)))
    private fun offerOf(s: ChatUiState) = s.messages.first { it.id == "r1" }.offer

    @Test fun answeringHidesTheOfferRemembersItAndSetsTheLine() {
        val s = OfferFlow.answered(chat(), "r1", OfferAnswer.Later)
        assertNull(offerOf(s))
        assertEquals(OfferResult.Later, s.offerResult)
        assertTrue("r1" in s.answeredOffers)
        assertNull(OfferFlow.answered(chat(), "r1", OfferAnswer.Yes).offerResult)
    }

    @Test fun aSecondTapOnTheSameOfferIsRefused() {
        assertTrue(OfferFlow.canAnswer(chat(), "r1"))
        val once = OfferFlow.answered(chat(), "r1", OfferAnswer.Yes)
        assertFalse(OfferFlow.canAnswer(once, "r1"))
        assertFalse(OfferFlow.canAnswer(chat(), "nope"))
    }

    @Test fun aReloadCannotBringAnAnsweredOfferBack() {
        val answered = OfferFlow.answered(chat(), "r1", OfferAnswer.NotNow)
        val reloaded = chat().messages
        assertNotNull(offerOf(chat()))
        assertNull(OfferFlow.strip(reloaded, answered.answeredOffers).first { it.id == "r1" }.offer)
        assertNotNull(OfferFlow.strip(reloaded, emptySet()).first { it.id == "r1" }.offer)
    }

    @Test fun aTransientFailurePutsNotNowAndLaterBackWithATryAgainLineAndNoLyingResult() {
        for (answer in listOf(OfferAnswer.NotNow, OfferAnswer.Later)) {
            for (status in listOf(0, 500, 503)) {
                val s = OfferFlow.failed(OfferFlow.answered(chat(), "r1", answer), "r1", offer, answer, status, t)
                assertEquals(offer, offerOf(s))
                assertNull(s.offerResult)
                assertFalse("r1" in s.answeredOffers)
                assertEquals(Notice(t.retry), s.notice)
                assertTrue(s.laterSupported)
            }
        }
    }

    @Test fun aFailedYesAlsoPutsTheOfferBackSoNothingIsSentTwice() {
        assertFalse(OfferFlow.yesProceeds(0))
        assertFalse(OfferFlow.yesProceeds(503))
        assertFalse(OfferFlow.yesProceeds(404))
        assertTrue(OfferFlow.yesProceeds(null))
        assertTrue(OfferFlow.yesProceeds(405))
        val s = OfferFlow.failed(OfferFlow.answered(chat(), "r1", OfferAnswer.Yes), "r1", offer, OfferAnswer.Yes, 0, t)
        assertEquals(offer, offerOf(s))
    }

    @Test fun a404DropsTheOfferForGoodWithAQuietLine() {
        val s = OfferFlow.failed(OfferFlow.answered(chat(), "r1", OfferAnswer.Later), "r1", offer, OfferAnswer.Later, 404, t)
        assertNull(offerOf(s))
        assertNull(s.offerResult)
        assertEquals(Notice(t.gone), s.notice)
        assertTrue("r1" in s.answeredOffers)
        assertTrue(s.laterSupported)
    }

    @Test fun onlyAnAbsentRouteHidesLaterAndA400DoesNot() {
        val asked = OfferFlow.answered(chat(), "r1", OfferAnswer.Later)
        val absent = OfferFlow.failed(asked, "r1", offer, OfferAnswer.Later, 405, t)
        assertFalse(absent.laterSupported)
        assertEquals(offer, offerOf(absent))
        val bad = OfferFlow.failed(asked, "r1", offer, OfferAnswer.Later, 400, t)
        assertTrue(bad.laterSupported)
        assertEquals(Notice(t.retry), bad.notice)
        assertEquals(offer, offerOf(bad))
    }

    @Test fun aConflictMarksLaterUsedOnTheRestoredOffer() {
        val s = OfferFlow.failed(OfferFlow.answered(chat(), "r1", OfferAnswer.Later), "r1", offer, OfferAnswer.Later, 409, t)
        assertTrue(offerOf(s)!!.laterUsed)
        assertEquals(listOf(OfferAnswer.Yes, OfferAnswer.NotNow), ChatSlot.answers(offerOf(s)!!, s.laterSupported))
    }

    // ---- pills

    private fun pills() = listOf(Suggestion("n", "A", "a", "a"), Suggestion("n", "B", "b", "b"))
    private fun withPills(l: List<Suggestion>?) = ChatUiState(suggestions = l, notice = Notice("other"))

    @Test fun removingAPillAnnouncesNothingUntilHqAnswers() {
        val s = PillFlow.removed(withPills(pills()), "a")
        assertEquals(listOf("b"), s.suggestions!!.map { it.id })
        assertEquals(Notice("other"), s.notice)
        assertEquals(Notice("Done"), PillFlow.confirmed(s, "Done").notice)
    }

    @Test fun aTransientFailureRestoresThePillAndSaysTryAgain() {
        val before = withPills(pills())
        val s = PillFlow.failed(PillFlow.removed(before, "a"), before, 503, t)
        assertEquals(pills(), s.suggestions)
        assertEquals(Notice(t.retry), s.notice)
        assertTrue(s.pillChoices)
    }

    @Test fun aMissingRouteRestoresThePillHidesTheMenuAndLeavesAnotherNoticeAlone() {
        val before = withPills(pills())
        for (status in listOf(404, 405, 501)) {
            val s = PillFlow.failed(PillFlow.removed(before, "a"), before, status, t)
            assertEquals(pills(), s.suggestions)
            assertFalse(s.pillChoices)
            assertEquals(Notice("other"), s.notice)
        }
    }

    @Test fun theLastHqPillGoneShowsNothingNotTheFixedThree() {
        val one = withPills(listOf(Suggestion("n", "A", "a", "a")))
        val s = PillFlow.removed(one, "a")
        assertTrue(s.pillsCleared)
        assertTrue(Suggestions.visible(s.suggestions, s.pillsCleared).isEmpty())
        assertEquals(Suggestions.Fallback, Suggestions.visible(null, false))
        val back = PillFlow.failed(s, one, 500, t)
        assertFalse(back.pillsCleared)
        assertEquals(1, Suggestions.visible(back.suggestions, back.pillsCleared).size)
    }
}
