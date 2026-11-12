package example.hq.spike.ui.shell

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The attention budget needs no Compose: it is state with rules (calm-UI report R1 to R3). */
class AttentionTest {
    private fun item(kind: AttentionKind, key: String = "") = AttentionItem(kind, key)

    @Test fun authorityOrderIsTheDocumentedOne() {
        val order = listOf(
            AttentionKind.Approval, AttentionKind.SignIn, AttentionKind.BlockingError, AttentionKind.ConnectionBanner,
            AttentionKind.UndoSnackbar, AttentionKind.HelperQuestion, AttentionKind.Offer, AttentionKind.Pills, AttentionKind.Digest,
        )
        assertEquals(order, AttentionKind.entries.filter { it in order })
    }

    @Test fun oneDecisionShowsAndTheBestRankedWins() {
        val b = AttentionBudget()
        b.post(item(AttentionKind.BlockingError))
        b.post(item(AttentionKind.SignIn))
        assertEquals(AttentionKind.SignIn, b.visible(AttentionSlot.Decision)?.kind)
        b.post(item(AttentionKind.Approval, "a"))
        assertEquals(AttentionKind.Approval, b.visible(AttentionSlot.Decision)?.kind)
        // The others are held, not dropped: they come back when the better one ends.
        b.clear(AttentionKind.Approval, "a")
        assertEquals(AttentionKind.SignIn, b.visible(AttentionSlot.Decision)?.kind)
    }

    @Test fun aLowerDecisionNeverReplacesAVisibleBetterOne() {
        val b = AttentionBudget()
        b.post(item(AttentionKind.Approval))
        b.post(item(AttentionKind.BlockingError))
        assertEquals(AttentionKind.Approval, b.visible(AttentionSlot.Decision)?.kind)
        assertEquals(2, b.posted().size)
    }

    @Test fun theBannerSlotNeverShowsTwo() {
        val b = AttentionBudget()
        b.post(item(AttentionKind.UpdateLine, "update"))
        b.post(item(AttentionKind.NotificationsBlocked))
        b.post(item(AttentionKind.ConnectionBanner, "connection"))
        assertEquals(AttentionKind.ConnectionBanner, b.visible(AttentionSlot.Banner)?.kind)
        b.clear(AttentionKind.ConnectionBanner, "connection")
        assertEquals(AttentionKind.NotificationsBlocked, b.visible(AttentionSlot.Banner)?.kind)
        b.clear(AttentionKind.NotificationsBlocked)
        assertEquals(AttentionKind.UpdateLine, b.visible(AttentionSlot.Banner)?.kind)
    }

    @Test fun theSameBannerPostedTwiceIsOneBanner() {
        val b = AttentionBudget()
        b.post(item(AttentionKind.ConnectionBanner, "connection"))
        b.post(item(AttentionKind.ConnectionBanner, "connection"))
        assertEquals(1, b.posted().size)
    }

    @Test fun aBannerClearsWhenItsStateEnds() {
        val b = AttentionBudget()
        b.post(item(AttentionKind.ConnectionBanner, "connection"))
        b.clear(AttentionKind.ConnectionBanner, "connection")
        assertNull(b.visible(AttentionSlot.Banner))
    }

    @Test fun aSecondSnackbarWaitsBehindTheFirst() {
        val b = AttentionBudget()
        b.post(item(AttentionKind.Snackbar, "one"))
        b.post(item(AttentionKind.UndoSnackbar, "two"))
        assertEquals("one", b.visible(AttentionSlot.Snackbar)?.key)
        assertEquals("two", b.queuedSnackbar()?.key)
        b.clear(AttentionKind.Snackbar, "one")
        assertEquals("two", b.visible(AttentionSlot.Snackbar)?.key)
        assertNull(b.queuedSnackbar())
    }

    @Test fun theQueueIsOneDeepSoTheNewestWaitingOneWins() {
        val b = AttentionBudget()
        b.post(item(AttentionKind.Snackbar, "one"))
        b.post(item(AttentionKind.Snackbar, "two"))
        b.post(item(AttentionKind.Snackbar, "three"))
        assertEquals("one", b.visible(AttentionSlot.Snackbar)?.key)
        assertEquals("three", b.queuedSnackbar()?.key)
        assertEquals(AttentionBudget.QUEUE_DEPTH + 1, b.posted().size)
    }

    @Test fun aSnackbarAndABannerAndADecisionCanShowTogetherButNotTwoOfAKind() {
        val b = AttentionBudget()
        b.post(item(AttentionKind.Approval))
        b.post(item(AttentionKind.ConnectionBanner, "connection"))
        b.post(item(AttentionKind.Snackbar, "s"))
        b.post(item(AttentionKind.Offer))
        b.post(item(AttentionKind.Digest))
        AttentionSlot.entries.forEach { assertTrue(it.name, b.visible(it) != null) }
        assertEquals(AttentionKind.Offer, b.visible(AttentionSlot.Inline)?.kind)
    }

    @Test fun aKindGetsItsNamedTimeAndAnUndoNeverLessThanThePhonesOwn() {
        val plain = item(AttentionKind.Snackbar)
        val undo = item(AttentionKind.UndoSnackbar)
        assertEquals(4_000L, AttentionBudget.shownFor(plain, 5_000L))
        assertEquals(10_000L, AttentionBudget.shownFor(undo, 5_000L))
        assertEquals(30_000L, AttentionBudget.shownFor(undo, 30_000L))
        assertEquals(1_500L, AttentionBudget.shownFor(plain.copy(timeoutMs = 1_500L), 5_000L))
        assertNull(AttentionBudget.shownFor(item(AttentionKind.ConnectionBanner), 5_000L))
    }

    @Test fun aPlainSnackbarNeverPushesOutAnUndoAndIsToldItWasDropped() {
        val b = AttentionBudget()
        var dropped = 0
        b.post(item(AttentionKind.Snackbar, "shown"))
        b.post(item(AttentionKind.UndoSnackbar, "undo"))
        b.post(AttentionItem(AttentionKind.Snackbar, "late", onExpire = { dropped++ }))
        assertEquals("undo", b.queuedSnackbar()?.key)
        assertEquals(1, dropped)
        assertEquals(2, b.posted().size)
    }

    @Test fun aPushedOutSnackbarHearsAboutItSoItsPosterCanLetGo() {
        val b = AttentionBudget()
        var pushed = 0
        b.post(item(AttentionKind.Snackbar, "shown"))
        b.post(AttentionItem(AttentionKind.UndoSnackbar, "old", onExpire = { pushed++ }))
        b.post(item(AttentionKind.UndoSnackbar, "new"))
        assertEquals("new", b.queuedSnackbar()?.key)
        assertEquals(1, pushed)
    }

    @Test fun postingTheSameItemAgainKeepsItsPlaceAndRestartsItsTime() {
        val b = AttentionBudget()
        b.post(item(AttentionKind.Snackbar, "a"))
        b.post(item(AttentionKind.Snackbar, "b"))
        val first = b.visible(AttentionSlot.Snackbar)!!.token
        b.post(item(AttentionKind.Snackbar, "a"))
        assertEquals("a", b.visible(AttentionSlot.Snackbar)?.key)
        assertTrue(b.visible(AttentionSlot.Snackbar)!!.token > first)
        assertEquals(2, b.posted().size)
    }

    @Test fun expiringTellsThePosterAndRemovesIt() {
        val b = AttentionBudget()
        var told = 0
        val i = AttentionItem(AttentionKind.Snackbar, "a", onExpire = { told++ })
        b.post(i)
        b.expire(i)
        b.expire(i)
        assertEquals(1, told)
        assertNull(b.visible(AttentionSlot.Snackbar))
    }

    @Test fun bannersAndDecisionsNeverTimeOut() {
        AttentionKind.entries.filter { it.slot == AttentionSlot.Banner || it.slot == AttentionSlot.Decision }
            .forEach { assertNull(it.name, AttentionBudget.timeoutMs(it)) }
    }
}
