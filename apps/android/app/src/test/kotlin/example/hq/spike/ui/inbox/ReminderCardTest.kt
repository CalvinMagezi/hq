package example.hq.spike.ui.inbox

import example.hq.spike.data.HqClient.InboxItem
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** A reminder HQ rang is something to do, not news: it lands in Needs you, first after a sign-in, and keeps the owner's own words. */
class ReminderCardTest {
    private fun ping(state: String = "pending", actions: List<String> = listOf("done", "snooze", "dismiss")) =
        InboxItem("val_r1", "value_item", "Call Moses about the 50,000", "Due 4:00 pm today.", state, "2026-11-27T13:00:00+00:00", "/reminders?id=abc", "reminder", "fyi", actions)

    @Test fun aRemindersPingIsAReminderCardNotAnUpdate() {
        val card = InboxWords.card(ping())
        assertEquals(CardKind.Reminder, card.kind)
        assertEquals("Call Moses about the 50,000", card.title)
        assertEquals("Due 4:00 pm today.", card.body)
        assertEquals("Reminder", card.where)
    }

    @Test fun anAnsweredReminderIsStillAReminderSoItsWordsHoldInHistory() {
        assertEquals(CardKind.Reminder, InboxWords.card(ping(state = "approved", actions = emptyList())).kind)
    }

    @Test fun aSimilarUpdateFromAnotherSourceIsStillAnUpdate() {
        val notice = InboxItem("val_n", "value_item", "Call Moses", "Due 4:00 pm today.", "pending", "2026-11-27T13:00:00+00:00", null, "agent_notify", "fyi", listOf("dismiss", "acknowledge"))
        assertEquals(CardKind.Update, InboxWords.card(notice).kind)
    }

    @Test fun remindersWaitInNeedsYouAheadOfHelpersAndUpdatesStayApart() {
        val helperAsk = InboxItem("val_h", "action_needed", "Harness session 'claude (mac)' is waiting for you", "Session s-1 on mac is blocked on a prompt or approval:\nDo you want to edit a file?", "pending", "2026-11-27T12:00:00+00:00", null, "session-supervisor", null, listOf("approve", "reject"))
        val update = InboxItem("val_u", "fyi", "Backup finished", "All good.", "pending", "2026-11-27T12:00:00+00:00", null, null, null, listOf("dismiss", "acknowledge"))
        val lists = inboxLists(listOf(helperAsk, ping(), update), emptyList())
        assertEquals(listOf(CardKind.Reminder, CardKind.HelperAsks), lists.needs.map { it.kind })
        assertTrue(lists.updates.none { it.kind == CardKind.Reminder })
    }
}
