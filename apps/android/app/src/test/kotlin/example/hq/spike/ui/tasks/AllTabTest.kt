package example.hq.spike.ui.tasks

import example.hq.spike.data.HqClient.TaskItem
import java.time.LocalDate
import java.time.LocalDateTime
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class AllTabTest {
    private val today = LocalDate.parse("2026-10-22")
    private val now = LocalDateTime.parse("2026-10-22T12:00:00")

    private fun t(id: String, status: String = "to_do", priority: String? = null, parent: String? = null) =
        TaskItem(id, "P-$id", "Task $id", "", status, priority, null, null, emptyList(), emptyList(), parent, 0, 0, null, null, "2026-10-20 10:00:00")

    @Test fun theActiveChipGroupsOpenTasksByStatus() {
        val g = TasksLogic.allList(listOf(t("1", "in_progress"), t("2", "blocked"), t("3", "ready_for_review"), t("4"), t("5", "complete")), AllChip.Active, emptyList())
        assertEquals(listOf("In progress", "Stuck", "Ready to check", "To do"), g.map { it.title })
    }

    @Test fun stuckAndReadyShowOnlyTheirOwnGroup() {
        val tasks = listOf(t("1", "blocked"), t("2", "ready_for_review"), t("3"))
        assertEquals(listOf("Stuck"), TasksLogic.allList(tasks, AllChip.Stuck, emptyList()).map { it.title })
        assertEquals(listOf("Ready to check"), TasksLogic.allList(tasks, AllChip.Review, emptyList()).map { it.title })
    }

    @Test fun doneIsOneListNewestFirstWithNoHeading() {
        val older = t("1", "complete").copy(completedAt = "2026-10-01 10:00:00")
        val newer = t("2", "complete").copy(completedAt = "2026-10-20 10:00:00")
        val sub = t("3", "complete", parent = "1")
        val g = TasksLogic.allList(listOf(older, newer, sub, t("4")), AllChip.Done, emptyList()).single()
        assertEquals(listOf("2", "1"), g.tasks.map { it.id })
        assertEquals("", g.title)
    }

    @Test fun archivedIsTheServersListNewestFirst() {
        val a = t("a").copy(archivedAt = "2026-10-01 10:00:00")
        val b = t("b").copy(archivedAt = "2026-10-21 10:00:00")
        assertEquals(listOf("b", "a"), TasksLogic.allList(emptyList(), AllChip.Archived, listOf(a, b)).single().tasks.map { it.id })
        assertEquals(emptyList<TaskGroup>(), TasksLogic.allList(emptyList(), AllChip.Archived, emptyList()))
    }

    @Test fun onlyStuckAndReadyCarryACount() {
        val tasks = listOf(t("1", "blocked"), t("2", "blocked"), t("3", "ready_for_review"), t("4", "blocked", parent = "9"))
        assertEquals(2, TasksLogic.chipCount(tasks, AllChip.Stuck))
        assertEquals(1, TasksLogic.chipCount(tasks, AllChip.Review))
        assertEquals(0, TasksLogic.chipCount(tasks, AllChip.Done))
    }

    @Test fun theTagOnTheFirstLineIsHowLongItIsStuckOrAHighPriority() {
        assertEquals(IdTag("Blocked 2d", true), TasksLogic.idTag(t("1", "blocked").copy(blockedSince = "2026-10-20 12:00:00"), now))
        assertEquals(IdTag("Blocked 5h", true), TasksLogic.idTag(t("1", "blocked").copy(blockedSince = "2026-10-22 07:00:00"), now))
        assertEquals(IdTag("Blocked", true), TasksLogic.idTag(t("1", "blocked"), now))
        assertEquals(IdTag("High", false), TasksLogic.idTag(t("2", priority = "high"), now))
        assertEquals(IdTag("Urgent", false), TasksLogic.idTag(t("2", priority = "urgent"), now))
        assertNull(TasksLogic.idTag(t("3", priority = "low"), now))
        assertNull(TasksLogic.idTag(t("4"), now))
    }

    @Test fun theThirdLineSaysWhatTheTaskIsWaitingOnOrWhoItIsFor() {
        assertEquals("Waiting on you \u00b7 needs a reboot window", TasksLogic.allMeta(t("1", "blocked").copy(waitingOn = "you", blockedReason = "needs a reboot window"), today, false, now))
        assertEquals("needs a reboot window", TasksLogic.allMeta(t("1", "blocked").copy(blockedReason = "needs a reboot window"), today, false, now))
        val ready = t("2", "ready_for_review").copy(assignees = listOf("hq"), updatedAt = "2026-10-22 09:00:00")
        assertEquals("for hq \u00b7 moved 3 h ago", TasksLogic.allMeta(ready, today, false, now))
    }

    @Test fun anOpenTaskNamesItsProgressDueDateAndOwner() {
        val task = t("3").copy(subtaskCount = 3, subtaskDone = 1, dueDate = "2026-10-24", assignees = listOf("hq"), tags = listOf("mobile-first"))
        assertEquals("hq is working \u00b7 1 of 3 sub-tasks \u00b7 due Saturday \u00b7 for hq \u00b7 mobile-first", TasksLogic.allMeta(task, today, true, now))
        assertEquals("", TasksLogic.allMeta(t("4"), today, false, now))
    }

    @Test fun doneAndArchivedRowsSayWhen() {
        assertEquals("Done 3 h ago", TasksLogic.allMeta(t("5", "complete").copy(completedAt = "2026-10-22 09:00:00"), today, false, now))
        assertEquals("Archived 2 d ago", TasksLogic.allMeta(t("6").copy(archivedAt = "2026-10-20 11:00:00"), today, false, now))
    }

    @Test fun eachChipHasWordsForAnEmptyList() {
        AllChip.entries.forEach { assertEquals(false, TasksLogic.emptyAll(it).isBlank()) }
    }
}
