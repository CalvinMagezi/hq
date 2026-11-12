package example.hq.spike.ui.inbox

import example.hq.spike.data.HqClient.InboxItem
import java.time.OffsetDateTime
import java.time.ZoneOffset
import kotlinx.coroutines.async
import kotlinx.coroutines.launch
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class QuietInboxTest {
    private fun at(minutes: Long) = OffsetDateTime.now(ZoneOffset.UTC).minusMinutes(minutes).toString()

    private fun item(id: String, minutes: Long, group: String? = null, ownerOnly: Boolean? = null, title: String = "Run $id?", state: String = "pending", expired: String? = null, source: String? = null, kind: String = "action_needed", body: String = "") =
        InboxItem(id, kind, title, body, state, at(minutes), null, source, null, listOf("approve", "reject"), ownerOnly = ownerOnly, sourceGroup = group, expiredReason = expired)

    private fun cards(vararg items: InboxItem) = items.map(InboxWords::card)

    @Test fun anOlderHqWithoutGroupsKeepsTheTabAsItWas() {
        val c = cards(item("a", 5), item("b", 9))
        assertFalse(NeedsLayout.grouped(c))
        assertTrue(c.none { it.fromHelpers })
        assertFalse(NeedsLayout.catchUp(c))
    }

    @Test fun theOldestOwnerDecisionIsTheHeroAndHelpersNeverAre() {
        val c = cards(item("h", 600, group = "helpers", ownerOnly = false), item("new", 5, group = "you", ownerOnly = true), item("old", 90, group = "you", ownerOnly = true))
        val l = NeedsLayout.of(c)
        assertEquals("old", l.hero?.id)
        assertEquals(listOf("new"), l.rows.map { it.id })
        assertEquals(listOf("h"), l.helpers.map { it.id })
    }

    @Test fun aHelperRequestThatOnlyTheOwnerCanDecideStaysAmongTheDecisions() {
        val c = cards(item("h", 600, group = "helpers", ownerOnly = true), item("y", 5, group = "you", ownerOnly = true))
        val l = NeedsLayout.of(c)
        assertEquals("h", l.hero?.id)
        assertTrue(l.helpers.isEmpty())
    }

    @Test fun fiveRowsThenTheRestFoldAndOpeningARowMovesTheHero() {
        val c = cards(*(1..9).map { item("d$it", it * 10L, group = "you", ownerOnly = true) }.toTypedArray())
        val l = NeedsLayout.of(c)
        assertEquals("d9", l.hero?.id)
        assertEquals(NEEDS_ROWS_MAX, l.rows.size)
        assertEquals(3, l.folded)
        assertEquals(0, NeedsLayout.of(c, showAll = true).folded)
        val opened = NeedsLayout.of(c, focusId = "d2")
        assertEquals("d2", opened.hero?.id)
        assertTrue(opened.rows.any { it.id == "d9" })
    }

    @Test fun theCatchUpCardNeedsMoreThanFiveHelperRequestsOverADayOld() {
        fun helpers(n: Int, hours: Long) = cards(*(1..n).map { item("h$it", hours * 60 + it, group = "helpers", ownerOnly = false) }.toTypedArray())
        val now = java.time.Instant.now()
        assertFalse(NeedsLayout.catchUp(helpers(6, 23), now))
        assertFalse(NeedsLayout.catchUp(helpers(5, 30), now))
        assertTrue(NeedsLayout.catchUp(helpers(6, 25), now))
        val ui = InboxUi(loaded = true, needs = helpers(6, 25))
        assertTrue(ui.catchUp)
        assertFalse(ui.copy(catchUpDismissed = true).catchUp)
    }

    @Test fun clearOldTakesOnlyHelperRequestsOlderThanADay() {
        val c = cards(item("young", 23 * 60, group = "helpers", ownerOnly = false), item("old", 25 * 60, group = "helpers", ownerOnly = false), item("mine", 50 * 60, group = "you", ownerOnly = true))
        assertEquals(listOf("old"), NeedsLayout.oldHelpers(c).map { it.id })
    }

    @Test fun helperMeansTheServerSaidHelpersAndNotOwnerOnly() {
        assertFalse(InboxWords.card(item("a", 1, group = "helpers", ownerOnly = null)).fromHelpers)
        assertTrue(InboxWords.card(item("b", 1, group = "helpers", ownerOnly = false)).fromHelpers)
        assertFalse(InboxWords.card(item("c", 1, group = "helpers", ownerOnly = true)).fromHelpers)
        assertFalse(InboxWords.card(item("d", 1, group = "you", ownerOnly = false)).fromHelpers)
    }

    @Test fun aSignInIsTheHeroEvenWhenNewerThanApprovals() {
        val signIn = item("si", 1, group = "you", ownerOnly = true, title = "A new device signed in").copy(actions = emptyList(), signInDevice = "dev1")
        val c = cards(*(1..9).map { item("d$it", it * 10L + 5, group = "you", ownerOnly = true) }.toTypedArray(), signIn)
        val l = NeedsLayout.of(c)
        assertEquals("si", l.hero?.id)
        assertTrue(NeedsLayout.of(c, showAll = false).rows.none { it.id == "si" })
        assertTrue(l.folded > 0)
    }

    @Test fun theBadgeCountsOnlyWhatHqCallsOwnerOnlyAndFallsBackToTheListOtherwise() {
        val needs = cards(item("a", 5), item("b", 6, group = "helpers", ownerOnly = false), item("c", 7, group = "helpers", ownerOnly = false))
        assertEquals(3, InboxUi(needs = needs).badge)
        assertEquals(1, InboxUi(needs = needs, ownerOnlyCount = 1).badge)
        assertEquals(0, InboxUi(needs = needs, ownerOnlyCount = 0).badge)
    }

    @Test fun tidiedItemsShowInDoneAsNoLongerWaitingAndCanBeRestored() {
        val lists = inboxLists(emptyList(), listOf(item("e", 30, group = "helpers", state = "expired", expired = "auto_tidy:session_ended"), item("d", 20, state = "dismissed")))
        val e = lists.done.single { it.id == "e" }
        assertTrue(e.expired)
        assertFalse(lists.done.single { it.id == "d" }.expired)
    }

    @Test fun twoUpdatesForOneEventFoldIntoOneRow() {
        val a = InboxWords.card(item("u1", 3, title = "Helper finished", body = "Done.", kind = "fyi"))
        val b = InboxWords.card(item("u2", 3, title = "Helper finished", body = "Done.", kind = "fyi"))
        val other = InboxWords.card(item("u3", 3, title = "Backup finished", body = "Saved.", kind = "fyi"))
        val folded = foldUpdates(listOf(a, b, other))
        assertEquals(listOf("u1", "u3"), folded.map { it.id })
        assertEquals(listOf("u1", "u2"), folded[0].ids)
        assertEquals(listOf("u3"), folded[1].ids)
    }

    @Test fun theFleetDigestIsOneUpdateRow() {
        val digestItem = item("dg", 5, title = "7 helpers finished today", body = "5 ready to check, 2 stopped\nclaude in hq-oss", source = "fleet-digest", kind = "fyi").copy(actions = emptyList())
        val digest = InboxWords.card(digestItem)
        assertTrue(digest.digest)
        assertEquals(CardKind.Update, digest.kind)
        assertEquals(1, inboxLists(listOf(digestItem), emptyList()).updates.size)
    }

    @Test fun theWholeSiteOfferShowsWithNoCountAndFromTheThirdAskWhenCounted() {
        fun card(n: Int?, site: String? = "example.com") = InboxWords.card(item("s", 1).copy(siteApprovals = n)).copy(site = site)
        assertTrue(card(null).offersWholeSite)
        assertFalse(card(0).offersWholeSite)
        assertFalse(card(1).offersWholeSite)
        assertTrue(card(2).offersWholeSite)
        assertFalse(card(5, site = null).offersWholeSite)
    }

    @Test fun theCountIsReadFromTheItemOrItsApprovalAndNullWhenAbsent() {
        fun n(json: String) = example.hq.spike.data.siteApprovals(org.json.JSONObject(json), org.json.JSONObject(json).optJSONObject("metadata"))
        assertNull(n("{}"))
        assertEquals(3, n("{\"site_approvals\":3}"))
        assertEquals(2, n("{\"metadata\":{\"approval\":{\"site_approvals\":2}}}"))
        assertEquals(0, n("{\"site_approvals\":0}"))
    }

    @Test fun theStackRuleStartsAtLargeText() {
        assertFalse(AnswerLayout.stacked(1.0f))
        assertFalse(AnswerLayout.stacked(1.29f))
        assertTrue(AnswerLayout.stacked(1.3f))
        assertTrue(AnswerLayout.stacked(2.0f))
    }

    @Test fun aClippedOrMultiLineUpdateCanAlwaysBeOpened() {
        assertFalse(UpdateRows.expandable("Short.", clipped = false))
        assertTrue(UpdateRows.expandable("Short.", clipped = true))
        assertTrue(UpdateRows.expandable("one\ntwo", clipped = false))
        assertFalse(UpdateRows.expandable("", clipped = false))
    }

    @Test fun everyToolHqAsksAboutGetsASentenceAndNoSnakeCaseName() {
        val tools = listOf(
            "calendar_delete_event", "calendar_create_event", "calendar_update_event", "calendar_list_events", "contacts_search",
            "bash", "web_fetch", "task_delete", "vault_import", "harness_session_send", "some_new_tool", "",
        )
        for (t in tools) {
            val s = InboxWords.toolSentence(t)
            assertTrue(s, s.startsWith("HQ wants to "))
            assertFalse(s, Regex("[a-z]+_[a-z]+").containsMatchIn(s))
        }
        assertEquals("HQ wants to delete a calendar event", InboxWords.toolSentence("calendar_delete_event"))
        assertEquals("HQ wants to use a tool", InboxWords.toolSentence("some_new_tool"))
    }

    @Test fun cardsAndDoneRowsUseTheSentenceNotTheToolName() {
        val approval = item("t", 5, title = "HQ wants to run calendar_delete_event")
        assertEquals("HQ wants to delete a calendar event", InboxWords.card(approval).title)
        val done = inboxLists(emptyList(), listOf(approval.copy(state = "approved"))).done.single()
        assertEquals("Allowed: HQ wants to delete a calendar event", done.title)
        assertEquals("Run it?", InboxWords.plainTitle("Run it?"))
    }

    // ---- the clear queue

    private fun card(id: String) = InboxWords.card(item(id, 1, group = "helpers", ownerOnly = false))

    @Test fun aSecondClearWaitsForTheFirstAndEachBatchKeepsItsOwnRecord() = kotlinx.coroutines.runBlocking {
        val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
        val order = mutableListOf<String>()
        val restored = mutableListOf<String>()
        val q = ClearQueue({ id -> if (id == "a") gate.await(); order += id }, { id -> restored += id; example.hq.spike.ui.shell.UndoResult.Done })
        val first = q.begin(listOf(card("a"), card("b")))
        val second = q.begin(listOf(card("c")))
        val j1 = launch { q.run(first) }
        val j2 = launch { q.run(second) }
        kotlinx.coroutines.yield()
        assertTrue("the second clear must wait", order.isEmpty())
        gate.complete(Unit)
        j1.join(); j2.join()
        assertEquals(listOf("a", "b", "c"), order)
        val undone = q.undo()!!
        assertEquals(listOf("c"), restored)
        assertEquals(listOf("c"), undone.cards.map { it.id })
        q.undo()
        assertEquals(listOf("c", "a", "b"), restored)
        assertNull(q.undo())
    }

    @Test fun undoStopsABatchStillRunningAndRestoresOnlyWhatWasDismissed() = kotlinx.coroutines.runBlocking {
        val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
        val dismissed = mutableListOf<String>()
        val restored = mutableListOf<String>()
        val q = ClearQueue({ id -> dismissed += id; if (id == "a") gate.await() }, { id -> restored += id; example.hq.spike.ui.shell.UndoResult.Done })
        val batch = q.begin(listOf(card("a"), card("b"), card("c")))
        val run = launch { q.run(batch) }
        kotlinx.coroutines.yield()
        val undo = async { q.undo() }
        kotlinx.coroutines.yield()
        gate.complete(Unit)
        run.join()
        val undone = undo.await()!!
        assertEquals(listOf("a"), dismissed)
        assertEquals(listOf("a"), restored)
        assertEquals(3, undone.cards.size)
        assertFalse(undone.late)
    }

    @Test fun aFoldedRowClearsAndRestoresEveryIdItCarries() = kotlinx.coroutines.runBlocking {
        val dismissed = mutableListOf<String>()
        val restored = mutableListOf<String>()
        val q = ClearQueue({ dismissed += it }, { restored += it; example.hq.spike.ui.shell.UndoResult.TooLate })
        val row = card("u1").copy(alsoIds = listOf("u2", "u3"))
        q.run(q.begin(listOf(row)))
        assertEquals(listOf("u1", "u2", "u3"), dismissed)
        assertTrue(q.undo()!!.late)
        assertEquals(dismissed, restored)
    }

    @Test fun restoringACardOutOfDoneCallsTheNotificationRestoreAndReportsTooLate() = kotlinx.coroutines.runBlocking {
        val seen = mutableListOf<String>()
        val ok = object : example.hq.spike.ui.shell.UndoRestore {
            override suspend fun unarchiveThread(id: String) {}
            override suspend fun restoreNotification(id: String) { seen += id }
            override suspend fun restoreTask(id: String) {}
            override suspend fun restoreBudgets(limits: org.json.JSONObject) {}
        }
        assertEquals(example.hq.spike.ui.shell.UndoResult.Done, example.hq.spike.ui.shell.Undo.restore(ok, example.hq.spike.ui.shell.UndoKind.Notification, "val_1"))
        assertEquals(listOf("val_1"), seen)
        val late = object : example.hq.spike.ui.shell.UndoRestore by ok {
            override suspend fun restoreNotification(id: String) { throw example.hq.spike.data.HqUnavailable("HTTP 409", status = 409) }
        }
        assertEquals(example.hq.spike.ui.shell.UndoResult.TooLate, example.hq.spike.ui.shell.Undo.restore(late, example.hq.spike.ui.shell.UndoKind.Notification, "val_1"))
    }
}
