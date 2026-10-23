package example.hq.spike.bridge

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class BridgeReducerTest {
    private val now = 1_000_000L
    private fun fold(vararg events: BridgeEvent, from: BridgeState = BridgeState(enabled = true)) =
        events.fold(from) { s, e -> BridgeReducer.reduce(s, e, now) }

    @Test fun readyMarksTheBridgeRunning() {
        val s = fold(BridgeEvent.Ready("aa", "emu", "default"))
        assertTrue(s.running)
        assertEquals("aa", s.deviceId)
    }

    @Test fun aTicketCountsDownFromItsLifetime() {
        val s = fold(BridgeEvent.Ticket("hqpair://ticket/x", 60))
        assertEquals(now + 60_000, s.ticket?.endsAt)
    }

    @Test fun aRequestThenAPairedDecisionClosesTheWaitAndRemembersHowItEnded() {
        val s = fold(
            BridgeEvent.Ticket("l", 60), BridgeEvent.Request("j", "Pixel", 120),
            BridgeEvent.Decision("j", "paired"), BridgeEvent.TicketClosed(true),
        )
        assertNull(s.request)
        assertNull(s.ticket)
        assertEquals(TicketEnd.Paired, s.ticketEnd)
    }

    @Test fun wrongDigitsAndLateAnswersAreTold() {
        assertEquals(TicketEnd.Declined, fold(BridgeEvent.Decision("j", "declined")).ticketEnd)
        assertEquals(TicketEnd.TooLate, fold(BridgeEvent.Decision("j", "too_late")).ticketEnd)
    }

    @Test fun aTicketThatRunsOutIsExpiredNotPaired() {
        assertEquals(TicketEnd.Expired, fold(BridgeEvent.Ticket("l", 60), BridgeEvent.TicketClosed(false)).ticketEnd)
    }

    @Test fun aNewTicketForgetsHowTheLastOneEnded() {
        val s = fold(BridgeEvent.TicketClosed(false), BridgeEvent.Ticket("l", 60))
        assertNull(s.ticketEnd)
        assertNotNull(s.ticket)
    }

    @Test fun joiningShowsTheDigitsThenTheResult() {
        val shown = fold(BridgeEvent.Digits("123456"))
        assertEquals(JoinState.ShowDigits("123456"), shown.join)
        assertEquals(JoinState.Done("aa", "Desk"), fold(BridgeEvent.Joined("aa", "Desk"), from = shown).join)
        assertEquals(JoinState.Failed("no"), fold(BridgeEvent.JoinFailed("no"), from = shown).join)
    }

    @Test fun revokingTakesThePeerOffTheListAtOnce() {
        val peers = listOf(BridgePeer("aa", "A", 1, null, null), BridgePeer("bb", "B", 2, null, null))
        val s = fold(BridgeEvent.Peers(peers), BridgeEvent.Revoked("aa", true))
        assertEquals(listOf("bb"), s.peers?.map { it.id })
    }

    @Test fun whenTheChildEndsNothingItShowedIsStillTrue() {
        val s = BridgeReducer.stopped(fold(BridgeEvent.Ready("aa", "n", "r"), BridgeEvent.Ticket("l", 60), BridgeEvent.Request("j", "P", 1)))
        assertEquals(false, s.running)
        assertNull(s.ticket)
        assertNull(s.request)
    }

    @Test fun aFailedOpenEndsTheWaitForACodeAndAFailedJoinEndsTheJoin() {
        val open = fold(BridgeEvent.Failed("open", "could not bind"))
        assertEquals(TicketEnd.Failed, open.ticketEnd)
        assertNull(open.ticket)
        assertEquals(JoinState.Failed("no route"), fold(BridgeEvent.Failed("join", "no route")).join)
        assertEquals("busy", fold(BridgeEvent.Failed("digits", "busy")).line)
    }

    @Test fun aChildThatEndsTakesItsListAndAJoinInFlightWithIt() {
        val s = BridgeReducer.stopped(fold(BridgeEvent.Peers(listOf(BridgePeer("aa", "A", 1, null, null))), BridgeEvent.Digits("123456")))
        assertNull(s.peers)
        assertEquals(JoinState.Failed(BridgeReducer.BRIDGE_STOPPED), s.join)
        assertEquals(JoinState.Done("aa", "A"), BridgeReducer.stopped(fold(BridgeEvent.Joined("aa", "A"))).join)
    }

    @Test fun theEndOfATicketHasTheLastWordOnWhetherADevicePaired() {
        // "Too late" was said, and then the device turned out to be on the list.
        val s = fold(BridgeEvent.Decision("j", "too_late"), BridgeEvent.TicketClosed(true))
        assertEquals(TicketEnd.Paired, s.ticketEnd)
        assertEquals(TicketEnd.TooLate, fold(BridgeEvent.Decision("j", "too_late"), BridgeEvent.TicketClosed(false)).ticketEnd)
    }
}
