package example.hq.spike.ui.inbox

import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class ClearWordsTest {
    private fun card(kind: CardKind) = InboxCard("1", kind, "t", "", "", "2026-01-01T00:00:00Z", CardIcon.Info, null, false, "", null, "", "pending")

    private fun line(kind: CardKind, n: Int): String {
        val cards = List(n) { card(kind) }
        return RuntimeEnvironment.getApplication().resources.getQuantityString(ClearWords.plural(cards), n, n)
    }

    @Test fun helperClearsNameHelperRequestsNeverAnEmptyLine() {
        assertEquals("1 helper request cleared", line(CardKind.Approval, 1))
        assertEquals("2 helper requests cleared", line(CardKind.HelperAsks, 2))
        assertEquals("40 helper requests cleared", line(CardKind.HelperAsks, 40))
    }

    @Test fun updateClearsNameUpdates() {
        assertEquals("1 update cleared", line(CardKind.Update, 1))
        assertEquals("2 updates cleared", line(CardKind.Update, 2))
        assertEquals("40 updates cleared", line(CardKind.Update, 40))
    }
}
