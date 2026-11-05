package example.hq.spike.ui.tasks

import example.hq.spike.testing.testWords

import example.hq.spike.data.HqClient.TaskItem
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class BulkEditTest {
    @org.junit.Before fun english() = java.util.Locale.setDefault(java.util.Locale.ENGLISH)

    private fun t(status: String = "to_do", priority: String? = null) =
        TaskItem("i", "FR-1", "T", "", status, priority, null, null, emptyList(), emptyList(), null, 0, 0, null, null, "2026-10-20 10:00:00")

    @Test fun aStatusChangeCarriesTheStatusItWasReadWith() {
        val body = BulkEdit.patchFor(Bulk.Status("complete"), t("in_progress"))!!
        assertEquals("complete", body.getString("status"))
        assertEquals("in_progress", body.getString("expected_status"))
    }

    @Test fun aTaskAlreadyThereIsLeftAlone() {
        assertNull(BulkEdit.patchFor(Bulk.Status("complete"), t("complete")))
        assertNull(BulkEdit.patchFor(Bulk.Priority("high"), t(priority = "high")))
        assertNull(BulkEdit.patchFor(Bulk.Priority(null), t(priority = null)))
    }

    @Test fun nonePriorityIsSentAsNull() {
        assertEquals(true, BulkEdit.patchFor(Bulk.Priority(null), t(priority = "high"))!!.isNull("priority"))
        assertEquals("urgent", BulkEdit.patchFor(Bulk.Priority("urgent"), t())!!.getString("priority"))
    }

    @Test fun theSummaryCountsWhatMovedWhatWasAlreadyThereAndWhatFailed() {
        val r = listOf(Applied.Changed, Applied.Changed, Applied.Changed, Applied.Already)
        assertEquals("3\u00A0tasks moved to Done, 1 already there", BulkEdit.summary(testWords(), Bulk.Status("complete"), r))
        val failed = r + Applied.Failed("FR-12", BulkEdit.failure(409)) + Applied.Failed("FR-13", BulkEdit.failure(404))
        assertEquals("3\u00A0tasks moved to Done, 1 already there. 2 could not be updated: FR-12 was changed by someone else; FR-13 is not there any more", BulkEdit.summary(testWords(), Bulk.Status("complete"), failed))
    }

    @Test fun theSummaryReadsForOneTaskAndForPriorities() {
        assertEquals("1\u00A0task set to High", BulkEdit.summary(testWords(), Bulk.Priority("high"), listOf(Applied.Changed)))
        assertEquals("2\u00A0tasks set to no priority", BulkEdit.summary(testWords(), Bulk.Priority(null), listOf(Applied.Changed, Applied.Changed)))
        assertEquals("2\u00A0tasks already there", BulkEdit.summary(testWords(), Bulk.Status("blocked"), listOf(Applied.Already, Applied.Already)))
        assertEquals("1\u00A0task already that", BulkEdit.summary(testWords(), Bulk.Priority("low"), listOf(Applied.Already)))
    }

    @Test fun everyStatusHasWordsAndPriorityEndsWithNone() {
        BulkEdit.STATUS_CHOICES.forEach { assertEquals(false, TasksLogic.statusWords(testWords(), it) == it) }
        assertEquals("None", BulkEdit.PRIORITY_CHOICES.last().second)
    }

    @Test fun movingToReadyToCheckSaysTheyAreWaitingInTheInbox() {
        assertEquals("2\u00A0tasks moved to Ready to check. Each is waiting in your Inbox", BulkEdit.summary(testWords(), Bulk.Status("ready_for_review"), listOf(Applied.Changed, Applied.Changed)))
    }

    @Test fun finishingManyAsksFirst() {
        assertEquals(BulkEdit.ConfirmWords("Mark 3 tasks done?", "Anything that was waiting on them is told it can start.", "Mark 3 tasks done"), BulkEdit.confirmDone(3))
        assertEquals("could not be saved", BulkEdit.failure(500))
        assertEquals("could not be reached", BulkEdit.failure(0))
    }
}
