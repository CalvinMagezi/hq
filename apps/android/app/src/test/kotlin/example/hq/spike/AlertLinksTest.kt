package example.hq.spike

import example.hq.spike.ui.inbox.CardIcon
import example.hq.spike.ui.inbox.CardKind
import example.hq.spike.ui.inbox.InboxCard
import example.hq.spike.ui.shell.LinkTarget
import example.hq.spike.ui.shell.Links
import example.hq.spike.ui.shell.Place
import org.junit.Assert.assertEquals
import org.junit.Test

/** A notification's link is read back by the same parser the app uses, so a tap opens what the notice is about. */
class AlertLinksTest {
    private fun card(kind: CardKind, id: String, sessionId: String? = null) =
        InboxCard(id, kind, "t", "b", "", "", CardIcon.Info, sessionId, false, "", null, "", "")

    @Test fun anApprovalOpensItsInboxCard() {
        assertEquals(LinkTarget(Place.Inbox, id = "val_9f3a"), Links.parse(AlertPoster.linkFor(card(CardKind.Approval, "val_9f3a"))))
    }

    @Test fun aHelpersQuestionOpensTheHelper() {
        assertEquals(LinkTarget(Place.Helpers, id = "s-42"), Links.parse(AlertPoster.linkFor(card(CardKind.HelperAsks, "val_1", "s-42"))))
    }

    @Test fun anUpdateOpensTheInbox() {
        assertEquals(LinkTarget(Place.Inbox), Links.parse(AlertPoster.linkFor(card(CardKind.Update, "val_2"))))
    }
}
