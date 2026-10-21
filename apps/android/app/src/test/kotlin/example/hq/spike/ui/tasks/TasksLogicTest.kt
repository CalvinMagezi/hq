package example.hq.spike.ui.tasks

import example.hq.spike.data.HqClient.TaskItem
import java.time.LocalDate
import java.time.LocalDateTime
import java.util.Locale
import org.junit.Before
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TasksLogicTest {
    private val today = LocalDate.of(2026, 10, 21)

    @Before fun words() = Locale.setDefault(Locale.ENGLISH)

    private fun task(
        id: String, status: String = "to_do", priority: String? = null, due: String? = null, start: String? = null, updated: String = "2026-10-01 10:00:00",
        reason: String? = null, readyAt: String? = null, steps: Pair<Int, Int> = 0 to 0, parent: String? = null,
    ) = TaskItem(id, "T-$id", "Task $id", "", status, priority, due, start, emptyList(), emptyList(), parent, steps.second, steps.first, reason, readyAt, updated)

    @Test fun todayHoldsWhatWaitsOnYouWhatIsGoingAndWhatIsDue() {
        val tasks = listOf(
            task("a", "blocked"), task("b", "ready_for_review"), task("c", "in_progress"), task("d", due = "2026-10-21"), task("e", start = "2026-10-20"),
            task("f", due = "2026-10-30"), task("g", "complete"), task("h", parent = "a"),
        )
        val groups = TasksLogic.today(tasks, today)
        assertEquals(listOf("Waiting on you", "In progress", "Due or starting today"), groups.map { it.title })
        assertEquals(listOf("a", "b"), groups[0].tasks.map { it.id }.sorted())
        assertEquals(listOf("d", "e"), groups[2].tasks.map { it.id }.sorted())
    }

    @Test fun priorityThenOldestOrdersARow() {
        val tasks = listOf(task("old", updated = "2026-09-01 00:00:00"), task("high", priority = "high", updated = "2026-10-10 00:00:00"), task("new", updated = "2026-10-11 00:00:00"))
        assertEquals(listOf("high", "old", "new"), tasks.sortedWith(TasksLogic.byUrgency).map { it.id })
    }

    @Test fun upcomingGroupsByDayThenLaterThenNoDate() {
        val tasks = listOf(
            task("a", start = "2026-10-22"), task("b", due = "2026-10-23"), task("c", due = "2026-10-23"), task("d", due = "2026-12-01"), task("e"), task("f", "in_progress", due = "2026-10-21"),
        )
        val g = TasksLogic.upcoming(tasks, today)
        assertTrue(g[0].title.startsWith("Tomorrow · "))
        assertEquals(2, g[1].tasks.size)
        assertEquals("Later · 1 task", g[2].title)
        assertEquals("No date · 1 task", g[3].title)
        assertTrue(g.none { grp -> grp.tasks.any { it.id == "f" } })
    }

    @Test fun rowsSayStuckAndReadyInTheBoardsWords() {
        val stuck = TasksLogic.meta(task("a", "blocked", reason = "needs a reboot window"), today)
        assertEquals("Stuck · needs a reboot window", stuck.joinToString("") { it.text })
        assertTrue(stuck[0].amber && stuck[0].bold)
        val ready = TasksLogic.meta(task("b", "ready_for_review", readyAt = "2026-10-21 09:00:00"), today, LocalDateTime.of(2026, 10, 21, 12, 0))
        assertEquals("Ready to check · hq finished 3 h ago", ready.joinToString("") { it.text })
    }

    @Test fun anOrdinaryRowShowsPriorityDueAndSteps() {
        val m = TasksLogic.meta(task("c", "in_progress", priority = "high", due = "2026-10-24", steps = 1 to 3), today)
        assertEquals("High · due Saturday · 1 of 3 steps done", m.joinToString("") { it.text })
        assertEquals("due Oct 30", TasksLogic.dueWords("2026-10-30", today))
        assertEquals("due today", TasksLogic.dueWords("2026-10-21", today))
        assertNull(TasksLogic.dueWords(null, today))
    }

    @Test fun quickAddReadsWordsWhereverTheyStand() {
        val q = QuickAdd.parse("Check Samsung battery whitelist tomorrow !high #inbox", today)
        assertEquals("Check Samsung battery whitelist", q.title)
        assertEquals("high", q.priority)
        assertEquals(LocalDate.of(2026, 10, 22), q.startDate)
        assertEquals("inbox", q.initiative)
        assertEquals(listOf("Tomorrow", "High", "Inbox"), q.understood.map { it.label })
    }

    @Test fun nextWeekIsTheNextMondayAndOtherWordsStayInTheTitle() {
        val q = QuickAdd.parse("tell the team next week about it", today)
        assertEquals("tell the team about it", q.title)
        assertEquals(LocalDate.of(2026, 10, 26), q.startDate)
        assertEquals("Next week", q.understood.single().label)
        assertEquals("Call mum!", QuickAdd.parse("Call mum!", today).title)
        assertNull(QuickAdd.parse("plain", today).priority)
    }

    @Test fun aLongWaitingGroupKeepsEveryTaskButAsksToShowOnlyTheFirstFive() {
        val tasks = (1..74).map { task("r$it", "ready_for_review") }
        val waiting = TasksLogic.today(tasks, today).first()
        assertEquals(74, waiting.tasks.size)
        assertEquals(TasksLogic.WAITING_PREVIEW, waiting.preview)
        assertEquals(null, TasksLogic.today(listOf(task("c", "in_progress")), today).first().preview)
    }

    @Test fun aToDoOnTheTodayListSaysWhyItIsThere() {
        assertEquals("starts today", TasksLogic.startedWords(task("a", start = "2026-10-21", due = "2026-12-06"), today))
        assertEquals("started Oct 1", TasksLogic.startedWords(task("b", start = "2026-10-01"), today))
        assertNull(TasksLogic.startedWords(task("c", start = "2026-10-21", due = "2026-10-21"), today))
        assertNull(TasksLogic.startedWords(task("d", "in_progress", start = "2026-10-01"), today))
        assertNull(TasksLogic.startedWords(task("e", start = "2026-10-30"), today))
        assertNull(TasksLogic.startedWords(task("f"), today))
    }

    @Test fun theRowMetaCarriesThatReason() {
        val words = TasksLogic.meta(task("a", priority = "high", start = "2026-10-21", due = "2026-12-06"), today).joinToString("") { it.text }
        assertEquals("High · due Dec 6 · starts today", words)
    }
}
