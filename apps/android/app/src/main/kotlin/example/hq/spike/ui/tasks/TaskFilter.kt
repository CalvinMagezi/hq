package example.hq.spike.ui.tasks

import example.hq.spike.data.HqClient.TaskItem

data class TaskSpace(val id: String, val name: String)
data class TaskFolder(val id: String, val spaceId: String, val name: String)
data class TaskInitiative(val id: String, val spaceId: String, val folderId: String?, val name: String)

/** Where tasks live, as HQ keeps them: a task belongs to an initiative, an initiative to a folder (or straight to a space). */
data class TaskTree(val spaces: List<TaskSpace>, val folders: List<TaskFolder>, val initiatives: List<TaskInitiative>)

/** The part of the tree the All tab is narrowed to. */
sealed interface TaskScope {
    data object Everything : TaskScope
    data class Space(val id: String) : TaskScope
    data class Folder(val id: String) : TaskScope
    data class Initiative(val id: String) : TaskScope
}

/** One line of the filter sheet: how deep it sits, what it narrows to, and how many open tasks are in it. */
data class ScopeRow(val label: String, val scope: TaskScope, val count: Int, val depth: Int)

object TaskFilters {
    /** The ids of the initiatives a scope covers; null for everything. */
    private fun initiativeIds(tree: TaskTree, scope: TaskScope): Set<String>? = when (scope) {
        TaskScope.Everything -> null
        is TaskScope.Space -> tree.initiatives.filter { it.spaceId == scope.id }.mapTo(HashSet()) { it.id }
        is TaskScope.Folder -> tree.initiatives.filter { it.folderId == scope.id }.mapTo(HashSet()) { it.id }
        is TaskScope.Initiative -> setOf(scope.id)
    }

    /** Whether a task is inside the scope. Until the tree is known nothing is hidden, so a failed read never empties the list. */
    fun keeps(tree: TaskTree?, scope: TaskScope): (TaskItem) -> Boolean {
        if (tree == null || scope == TaskScope.Everything) return { true }
        val ids = initiativeIds(tree, scope) ?: return { true }
        return { it.initiativeId in ids }
    }

    /** "All tasks", then each space with its folders and initiatives under it, each with the open tasks it holds. Empty groups are left out. */
    fun rows(tree: TaskTree, tasks: List<TaskItem>): List<ScopeRow> {
        val open = tasks.filter { TasksLogic.open(it) }
        val perInitiative = open.groupingBy { it.initiativeId }.eachCount()
        fun count(ids: Collection<String>) = ids.sumOf { perInitiative[it] ?: 0 }
        val out = mutableListOf(ScopeRow("All tasks", TaskScope.Everything, open.size, 0))
        for (space in tree.spaces) {
            val inSpace = tree.initiatives.filter { it.spaceId == space.id }
            if (inSpace.isEmpty()) continue
            out += ScopeRow(space.name, TaskScope.Space(space.id), count(inSpace.map { it.id }), 1)
            for (folder in tree.folders.filter { it.spaceId == space.id }) {
                val inFolder = inSpace.filter { it.folderId == folder.id }
                if (inFolder.isEmpty()) continue
                out += ScopeRow(folder.name, TaskScope.Folder(folder.id), count(inFolder.map { it.id }), 2)
                inFolder.forEach { out += ScopeRow(it.name, TaskScope.Initiative(it.id), perInitiative[it.id] ?: 0, 3) }
            }
            inSpace.filter { it.folderId == null }.forEach { out += ScopeRow(it.name, TaskScope.Initiative(it.id), perInitiative[it.id] ?: 0, 2) }
        }
        return out
    }

    /** The name of what the list is narrowed to, for the bar; null for everything or when it is not in the tree. */
    fun label(tree: TaskTree?, scope: TaskScope): String? = when (scope) {
        TaskScope.Everything -> null
        is TaskScope.Space -> tree?.spaces?.firstOrNull { it.id == scope.id }?.name
        is TaskScope.Folder -> tree?.folders?.firstOrNull { it.id == scope.id }?.name
        is TaskScope.Initiative -> tree?.initiatives?.firstOrNull { it.id == scope.id }?.name
    }
}
