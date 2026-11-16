package example.hq.spike.ui.tasks

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import example.hq.spike.R
import example.hq.spike.data.HqClient.TaskItem
import example.hq.spike.data.HqUnavailable
import example.hq.spike.testing.testWords
import example.hq.spike.ui.shell.ConfirmCopy
import example.hq.spike.ui.shell.ConfirmRules
import java.time.LocalDateTime
import java.util.Locale
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class WaitingGroupsTest {
    private val now = LocalDateTime.of(2026, 11, 16, 12, 0, 0)

    @Before fun english() = Locale.setDefault(Locale.ENGLISH)

    private fun task(
        id: String,
        status: String = "ready_for_review",
        priority: String? = null,
        updated: String = "2026-10-01 10:00:00",
        initiativeId: String? = null,
    ) = TaskItem(
        id = id,
        displayId = "T-$id",
        title = "Task $id",
        description = "",
        status = status,
        priority = priority,
        dueDate = null,
        startDate = null,
        tags = emptyList(),
        assignees = emptyList(),
        parentId = null,
        subtaskCount = 0,
        subtaskDone = 0,
        blockedReason = null,
        readyAt = null,
        updatedAt = updated,
        initiativeId = initiativeId,
    )

    @Test fun agingIsOldRespectsFourteenDayBoundaryNullAndFutureDates() {
        assertFalse(Aging.isOld(null, now))
        assertFalse(Aging.isOld("", now))
        assertFalse(Aging.isOld("not-a-date", now))
        // Future date is never old.
        assertFalse(Aging.isOld("2026-11-17 12:00:00", now))
        // Exactly 14 days ago is not older than 14 days.
        assertFalse(Aging.isOld("2026-11-02 12:00:00", now))
        assertFalse(Aging.isOld("2026-11-03 12:00:00", now))
        // One second past 14 days is old.
        assertTrue(Aging.isOld("2026-11-02 11:59:59", now))
        assertTrue(Aging.isOld("2026-10-01T10:00:00Z", now))
    }

    @Test fun groupsByInitiativeOrdersByUrgencyThenRecentAndOpensFirstThree() {
        val tree = TaskTree(
            spaces = emptyList(),
            folders = emptyList(),
            initiatives = listOf(
                TaskInitiative("init-a", "s1", null, "Alpha"),
                TaskInitiative("init-b", "s1", null, "Beta"),
                TaskInitiative("init-c", "s1", null, "Gamma"),
            ),
        )
        val tasks = listOf(
            task("1", priority = "normal", updated = "2026-11-10 10:00:00", initiativeId = "init-a"),
            task("2", priority = "urgent", updated = "2026-10-01 10:00:00", initiativeId = "init-b"),
            task("3", priority = "normal", updated = "2026-11-12 10:00:00", initiativeId = "init-c"),
            task("4", priority = "low", updated = "2026-11-15 10:00:00", initiativeId = null),
            task("5", priority = "low", updated = "2026-11-14 10:00:00", initiativeId = "missing-long-id-1234"),
        )
        val groups = WaitingGroups.group(testWords(), tasks, tree, now)
        assertEquals(listOf("Beta", "Gamma", "Alpha", "No initiative", "missing-"), groups.map { it.title })
        assertTrue(groups[0].isOpen(emptyMap()))
        assertTrue(groups[1].isOpen(emptyMap()))
        assertTrue(groups[2].isOpen(emptyMap()))
        assertFalse(groups[3].isOpen(emptyMap()))
        assertFalse(groups[4].isOpen(emptyMap()))

        // Toggling flips open state.
        val toggled0 = WaitingGroups.toggleOpen(emptyMap(), groups[0].key, groups)
        val toggled3 = WaitingGroups.toggleOpen(emptyMap(), groups[3].key, groups)
        assertFalse(groups[0].isOpen(toggled0))
        assertTrue(groups[3].isOpen(toggled3))
    }

    @Test fun collapseStateSurvivesGroupReorder() {
        val initialTasks = listOf(
            task("a", priority = "urgent", initiativeId = "init-0"),
            task("b", priority = "high", initiativeId = "init-1"),
            task("c", priority = "normal", initiativeId = "init-2"),
            task("d", priority = "low", updated = "2026-11-15 10:00:00", initiativeId = "init-3"),
            task("e", priority = "low", updated = "2026-11-14 10:00:00", initiativeId = "init-4"),
        )
        val initial = WaitingGroups.group(testWords(), initialTasks, null, now)
        var openState = WaitingGroups.recordDefaults(emptyMap(), initial)
        // Collapse index-1 group (init-1); leave index-3 (init-3) collapsed; expand index-4 (init-4).
        openState = WaitingGroups.toggleOpen(openState, initial[1].key, initial)
        openState = WaitingGroups.toggleOpen(openState, initial[4].key, initial)

        // Reorder: init-4 and init-3 move into top 3; init-1 falls to index 4.
        val reorderedTasks = listOf(
            task("e", priority = "urgent", initiativeId = "init-4"),
            task("d", priority = "urgent", initiativeId = "init-3"),
            task("a", priority = "high", initiativeId = "init-0"),
            task("c", priority = "normal", initiativeId = "init-2"),
            task("b", priority = "low", initiativeId = "init-1"),
        )
        val reordered = WaitingGroups.group(testWords(), reorderedTasks, null, now)
        openState = WaitingGroups.recordDefaults(openState, reordered)

        val g1 = reordered.first { it.initiativeId == "init-1" }
        val g3 = reordered.first { it.initiativeId == "init-3" }
        val g4 = reordered.first { it.initiativeId == "init-4" }
        assertEquals(4, reordered.indexOf(g1))
        assertFalse(g1.isOpen(openState))
        assertFalse(g3.isOpen(openState))
        assertTrue(g4.isOpen(openState))
    }

    @Test fun shouldReadTreeRespectsCooldownAndUnknownInitiative() {
        val nowMs = 1_000_000L
        val tree = TaskTree(emptyList(), emptyList(), listOf(TaskInitiative("init-a", "s1", null, "Alpha")))
        assertTrue(WaitingGroups.shouldReadTree(null, null, nowMs, false))
        assertFalse(WaitingGroups.shouldReadTree(null, nowMs - 60_000L, nowMs, false))
        assertTrue(WaitingGroups.shouldReadTree(null, nowMs - WaitingGroups.TREE_RETRY_COOLDOWN_MS, nowMs, false))
        assertFalse(WaitingGroups.shouldReadTree(tree, null, nowMs, false))
        assertTrue(WaitingGroups.shouldReadTree(tree, null, nowMs, true))
        assertFalse(WaitingGroups.shouldReadTree(tree, nowMs - 60_000L, nowMs, true))

        assertFalse(WaitingGroups.hasUnknownInitiative(listOf(task("1", initiativeId = "init-a")), tree))
        assertTrue(WaitingGroups.hasUnknownInitiative(listOf(task("1", initiativeId = "init-new")), tree))
    }

    @Test fun eligibleForGroupRecomputesFromCurrentTaskList() {
        val current = listOf(
            task("still-old-ready", status = "ready_for_review", updated = "2026-10-01 10:00:00", initiativeId = "init-1"),
            task("became-blocked", status = "blocked", updated = "2026-10-01 10:00:00", initiativeId = "init-1"),
            task("became-fresh", status = "ready_for_review", updated = "2026-11-15 10:00:00", initiativeId = "init-1"),
            task("other-init", status = "ready_for_review", updated = "2026-10-01 10:00:00", initiativeId = "init-2"),
        )
        val eligible = WaitingGroups.eligibleForGroup(current, "init-1", now)
        assertEquals(listOf("still-old-ready"), eligible.map { it.id })
    }

    @Test fun summaryLineReportsPartialFailureMovedZeroAndAllSucceeded() {
        val w = testWords()
        val t1 = task("1")
        val t2 = task("2")
        val partial = BulkReviewOutcome(
            moved = listOf(t1, t2),
            results = listOf(Applied.Changed, Applied.Changed, Applied.Failed("T-3", BulkEdit.failure(409))),
        )
        assertEquals("Marked 2 reviewed, 1 could not be updated", WaitingBulkReview.summaryLine(w, partial))

        val noneMoved = BulkReviewOutcome(
            moved = emptyList(),
            results = listOf(Applied.Failed("T-1", BulkEdit.failure(409))),
        )
        assertEquals("1 could not be updated: T-1 was changed by someone else", WaitingBulkReview.summaryLine(w, noneMoved))

        val allGood = BulkReviewOutcome(
            moved = listOf(t1, t2),
            results = listOf(Applied.Changed, Applied.Changed),
        )
        assertEquals("Marked 2 reviewed", WaitingBulkReview.summaryLine(w, allGood))
    }

    @Test fun blockedOldTasksAreNeverIncludedInMarkOldReviewed() {
        val tasks = listOf(
            task("b1", status = "blocked", updated = "2026-09-01 10:00:00", initiativeId = "i1"),
            task("b2", status = "blocked", updated = "2026-09-02 10:00:00", initiativeId = "i1"),
            task("r1", status = "ready_for_review", updated = "2026-09-03 10:00:00", initiativeId = "i1"),
            task("r2", status = "ready_for_review", updated = "2026-11-15 10:00:00", initiativeId = "i1"),
        )
        val onlyOneOldReview = WaitingGroups.group(testWords(), tasks, null, now).single()
        assertFalse(onlyOneOldReview.canMarkOldReviewed)
        assertEquals(listOf("r1"), onlyOneOldReview.oldReviewable.map { it.id })

        val withTwoOldReview = tasks + task("r3", status = "ready_for_review", updated = "2026-09-04 10:00:00", initiativeId = "i1")
        val group = WaitingGroups.group(testWords(), withTwoOldReview, null, now).single()
        assertTrue(group.canMarkOldReviewed)
        assertEquals(listOf("r1", "r3"), group.oldReviewable.map { it.id }.sorted())
    }

    @Test fun undoUsesOnlyTheIdsThatActuallyChangedAndReportsPartialFailure() = runBlocking {
        val selected = listOf(
            task("ok1", status = "ready_for_review"),
            task("conflict", status = "ready_for_review"),
            task("ok2", status = "ready_for_review"),
        )
        val moveBodies = mutableMapOf<String, JSONObject>()
        val outcome = WaitingBulkReview.markReviewed(selected) { id, body ->
            if (id == "conflict") throw HqUnavailable("conflict", status = 409)
            moveBodies[id] = body
        }

        // Only ok1 and ok2 changed; conflict was skipped.
        assertEquals(setOf("ok1", "ok2"), outcome.moved.map { it.id }.toSet())
        assertEquals("complete", moveBodies["ok1"]!!.getString("status"))
        assertEquals("ready_for_review", moveBodies["ok1"]!!.getString("expected_status"))

        // Undo sends ready_for_review with expected_status complete ONLY for ok1 and ok2; ok2 fails meanwhile.
        val undoIds = mutableListOf<String>()
        val undoBodies = mutableMapOf<String, JSONObject>()
        val undoOutcome = WaitingBulkReview.undoReviewed(outcome.moved) { id, body ->
            undoIds += id
            undoBodies[id] = body
            if (id == "ok2") throw HqUnavailable("changed since", status = 409)
        }

        assertEquals(setOf("ok1", "ok2"), undoIds.toSet())
        assertFalse("conflict" in undoIds)
        assertEquals("ready_for_review", undoBodies["ok1"]!!.getString("status"))
        assertEquals("complete", undoBodies["ok1"]!!.getString("expected_status"))
        assertEquals(1, undoOutcome.restoredCount)
        assertEquals(1, undoOutcome.failedCount)

        val ctx = ApplicationProvider.getApplicationContext<Context>()
        assertEquals("1 could not be restored", ctx.getString(R.string.tasks_undo_partial_fail, undoOutcome.failedCount.toString()))
    }

    @Test fun confirmCopyMatchesTitleAndUsesPlainWords() {
        val ctx = ApplicationProvider.getApplicationContext<Context>()
        val words = ConfirmCopy.markOldReviewed(ctx, 5, "Mobile")
        assertTrue(words.title.contains("5") && words.title.contains("Mobile"))
        assertTrue(ConfirmRules.actMatchesTitle(words.title, words.act))
        assertFalse(words.title.contains('—'))
        assertFalse(words.body.contains('—'))
        assertFalse(words.act.contains('—'))
    }
}
