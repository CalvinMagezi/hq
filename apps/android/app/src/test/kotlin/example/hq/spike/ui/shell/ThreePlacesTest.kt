package example.hq.spike.ui.shell

import example.hq.spike.ui.inbox.CardIcon
import example.hq.spike.ui.inbox.CardKind
import example.hq.spike.ui.inbox.InboxCard
import example.hq.spike.ui.inbox.needsOfChat
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** The bar has Notes, Chat and Inbox; Tasks and Helpers still open from a link; a chat shows only its own requests. */
class ThreePlacesTest {
    private fun card(id: String, kind: CardKind, threadId: String?) =
        InboxCard(id, kind, "t", "b", "", "", CardIcon.Info, null, true, "Allow", null, "", "pending", threadId = threadId)

    @Test fun theBarHasThreePlacesWithChatInTheMiddle() {
        assertEquals(listOf(Place.Notes, Place.Chat, Place.Inbox), Place.bar)
    }

    @Test fun tasksAndHelpersStillOpenFromALink() {
        assertEquals(Place.Tasks, Links.parse("hq://tasks")!!.place)
        assertEquals(Place.Helpers, Links.parse("hq://helpers/s-1")!!.place)
    }

    @Test fun aChatShowsOnlyTheRequestsItAsked() {
        val needs = listOf(
            card("a", CardKind.Approval, "th-1"), card("b", CardKind.Approval, "th-2"),
            card("c", CardKind.Approval, null), card("d", CardKind.HelperAsks, "th-1"),
        )
        assertEquals(listOf("a"), needsOfChat(needs, "th-1").map { it.id })
        assertTrue(needsOfChat(needs, null).isEmpty())
    }

    @Test fun aSilencedRequestIsListedButLeftOffTheBadge() {
        val ui = example.hq.spike.ui.inbox.InboxUi(needs = listOf(card("a", CardKind.Approval, null), card("b", CardKind.HelperAsks, null).copy(silenced = true)))
        assertEquals(2, ui.shownNeeds.size)
        assertEquals(1, ui.badge)
    }

    @Test fun silencedItemsAreToldApartSoTheyAreRememberedWithoutANotice() {
        val items = linkedMapOf("a" to org.json.JSONObject("""{"silenced":true}"""), "b" to org.json.JSONObject("""{"silenced":false}"""), "c" to org.json.JSONObject("{}"))
        assertEquals(listOf("a"), example.hq.spike.AlertsSilenced.ids(items))
    }
}
