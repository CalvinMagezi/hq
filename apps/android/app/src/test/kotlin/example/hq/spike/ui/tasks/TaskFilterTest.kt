package example.hq.spike.ui.tasks

import example.hq.spike.data.HqClient.TaskItem
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class TaskFilterTest {
    private val tree = TaskTree(
        spaces = listOf(TaskSpace("personal", "Personal"), TaskSpace("work", "Professional"), TaskSpace("empty", "Unused")),
        folders = listOf(TaskFolder("f1", "work", "Clients")),
        initiatives = listOf(
            TaskInitiative("inbox", "personal", null, "Inbox"),
            TaskInitiative("a", "work", "f1", "Alpha"),
            TaskInitiative("b", "work", "f1", "Beta"),
            TaskInitiative("loose", "work", null, "Loose"),
        ),
    )

    private fun task(id: String, initiative: String?, status: String = "to_do", parent: String? = null) =
        TaskItem(id, "P-$id", "T$id", "", status, null, null, null, emptyList(), emptyList(), parent, 0, 0, null, null, "2026-10-20 10:00:00", initiativeId = initiative)

    private val tasks = listOf(task("1", "inbox"), task("2", "a"), task("3", "a"), task("4", "b", "complete"), task("5", "loose"), task("6", "a", parent = "2"))

    @Test fun everythingKeepsEveryTaskAndSoDoesAnUnknownTree() {
        assertEquals(6, tasks.count(TaskFilters.keeps(tree, TaskScope.Everything)))
        assertEquals(6, tasks.count(TaskFilters.keeps(null, TaskScope.Space("work"))))
    }

    @Test fun aSpaceAFolderAndAnInitiativeNarrowByWhereTheTaskLives() {
        assertEquals(listOf("2", "3", "4", "5", "6"), tasks.filter(TaskFilters.keeps(tree, TaskScope.Space("work"))).map { it.id })
        assertEquals(listOf("2", "3", "4", "6"), tasks.filter(TaskFilters.keeps(tree, TaskScope.Folder("f1"))).map { it.id })
        assertEquals(listOf("2", "3", "6"), tasks.filter(TaskFilters.keeps(tree, TaskScope.Initiative("a"))).map { it.id })
        assertEquals(emptyList<String>(), tasks.filter(TaskFilters.keeps(tree, TaskScope.Initiative("gone"))).map { it.id })
    }

    @Test fun theSheetListsEachNodeWithItsOpenTopLevelTasksAndSkipsEmptySpaces() {
        val rows = TaskFilters.rows(tree, tasks)
        assertEquals(
            listOf("All tasks 4 0", "Personal 1 1", "Inbox 1 2", "Professional 3 1", "Clients 2 2", "Alpha 2 3", "Beta 0 3", "Loose 1 2"),
            rows.map { "${it.label} ${it.count} ${it.depth}" },
        )
        assertEquals(TaskScope.Folder("f1"), rows.first { it.label == "Clients" }.scope)
    }

    @Test fun theBarNamesWhatTheListIsNarrowedTo() {
        assertNull(TaskFilters.label(tree, TaskScope.Everything))
        assertEquals("Clients", TaskFilters.label(tree, TaskScope.Folder("f1")))
        assertNull(TaskFilters.label(null, TaskScope.Space("work")))
        assertEquals("12 active", TaskFilterWords.bar(12, null))
        assertEquals("12 active · Alpha", TaskFilterWords.bar(12, "Alpha"))
    }
}
