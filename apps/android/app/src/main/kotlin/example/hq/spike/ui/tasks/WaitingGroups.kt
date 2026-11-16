package example.hq.spike.ui.tasks

import example.hq.spike.R
import example.hq.spike.data.HqClient.TaskItem
import example.hq.spike.data.HqUnavailable
import example.hq.spike.data.Instants
import example.hq.spike.ui.shell.Words
import java.time.LocalDateTime
import java.time.ZoneOffset
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import org.json.JSONObject

/** Age checks for tasks based on their last update time. */
object Aging {
    const val OLD_AFTER_DAYS = 14L

    /** True when [updatedAt] is strictly older than [OLD_AFTER_DAYS] days before [now]; null, unreadable, or future dates are false. */
    fun isOld(updatedAt: String?, now: LocalDateTime = LocalDateTime.now(ZoneOffset.UTC)): Boolean {
        val then = Instants.parseUtc(updatedAt) ?: return false
        if (then.isAfter(now)) return false
        return then.isBefore(now.minusDays(OLD_AFTER_DAYS))
    }
}

/** One initiative sub-group inside the Waiting section. */
data class WaitingGroup(
    val key: String,
    val initiativeId: String?,
    val title: String,
    val tasks: List<TaskItem>,
    val oldReviewable: List<TaskItem>,
    val defaultOpen: Boolean,
) {
    /** Key for expanding the 5-row preview to all tasks in this group. */
    val allKey: String get() = "$key:all"

    fun isOpen(openState: Map<String, Boolean>): Boolean = openState[key] ?: defaultOpen

    fun isAllShown(expanded: Set<String>): Boolean = allKey in expanded

    val canMarkOldReviewed: Boolean get() = oldReviewable.size >= WaitingGroups.MIN_OLD_TO_REVIEW
}

/** Sub-grouping and eligibility rules for the Waiting on you list. */
object WaitingGroups {
    const val DEFAULT_OPEN_COUNT = 3
    const val MIN_OLD_TO_REVIEW = 2
    const val SHORT_ID_CHARS = 8
    const val UNDO_REVIEWED_MS = 8_000L
    const val TREE_RETRY_COOLDOWN_MS = 5 * 60 * 1000L

    private val PRIORITY_ORDER = listOf("urgent", "high", "normal", "low")

    private fun priorityRank(p: String?): Int =
        PRIORITY_ORDER.indexOf(p).let { if (it < 0) PRIORITY_ORDER.size else it }

    private fun mostRecentMs(tasks: List<TaskItem>): Long =
        tasks.maxOfOrNull { Instants.parse(it.updatedAt)?.toEpochMilli() ?: Long.MIN_VALUE } ?: Long.MIN_VALUE

    /** Resolves an initiative name from [tree], falling back to a short id or "No initiative". */
    fun initiativeTitle(w: Words, initiativeId: String?, tree: TaskTree?): String {
        val id = initiativeId?.takeIf { it.isNotBlank() } ?: return w.s(R.string.tasks_no_initiative)
        return tree?.initiatives?.firstOrNull { it.id == id }?.name?.takeIf { it.isNotBlank() }
            ?: id.take(SHORT_ID_CHARS)
    }

    /** Only ready_for_review tasks older than [Aging.OLD_AFTER_DAYS] qualify; blocked tasks wait on someone else, not on a review. */
    fun oldReviewable(tasks: List<TaskItem>, now: LocalDateTime): List<TaskItem> =
        tasks.filter { it.status == "ready_for_review" && Aging.isOld(it.updatedAt, now) }

    /**
     * Groups [waitingTasks] by initiative, ordered by most urgent task then most recent update.
     * The first [DEFAULT_OPEN_COUNT] groups start open; the rest start collapsed.
     */
    fun group(
        w: Words,
        waitingTasks: List<TaskItem>,
        tree: TaskTree?,
        now: LocalDateTime = LocalDateTime.now(ZoneOffset.UTC),
    ): List<WaitingGroup> {
        if (waitingTasks.isEmpty()) return emptyList()
        val byInit = waitingTasks.groupBy { it.initiativeId?.takeIf { id -> id.isNotBlank() } }
        val sorted = byInit.entries.sortedWith(
            compareBy<Map.Entry<String?, List<TaskItem>>> { (_, ts) -> ts.minOf { priorityRank(it.priority) } }
                .thenByDescending { (_, ts) -> mostRecentMs(ts) }
                .thenBy { (id, _) -> initiativeTitle(w, id, tree).lowercase() },
        )
        return sorted.mapIndexed { idx, (id, ts) ->
            val tasks = ts.sortedWith(TasksLogic.byUrgency)
            WaitingGroup(
                key = "waiting-init-${id ?: "none"}",
                initiativeId = id,
                title = initiativeTitle(w, id, tree),
                tasks = tasks,
                oldReviewable = oldReviewable(tasks, now),
                defaultOpen = idx < DEFAULT_OPEN_COUNT,
            )
        }
    }

    /** Records the first-seen default for any group not yet in [seen] so later reorders never flip it. */
    fun recordDefaults(seen: Map<String, Boolean>, groups: List<WaitingGroup>): Map<String, Boolean> {
        if (groups.all { it.key in seen }) return seen
        val out = seen.toMutableMap()
        for (g in groups) if (g.key !in out) out[g.key] = g.defaultOpen
        return out
    }

    /** Flips the explicit open state for [key], seeded from its first-seen or current default. */
    fun toggleOpen(openState: Map<String, Boolean>, key: String, groups: List<WaitingGroup>): Map<String, Boolean> {
        val current = openState[key] ?: groups.firstOrNull { it.key == key }?.defaultOpen ?: false
        return openState + (key to !current)
    }

    fun hasUnknownInitiative(tasks: List<TaskItem>, tree: TaskTree?): Boolean {
        if (tree == null) return false
        val known = tree.initiatives.mapTo(HashSet()) { it.id }
        return tasks.any { !it.initiativeId.isNullOrBlank() && it.initiativeId !in known }
    }

    fun shouldReadTree(
        tree: TaskTree?,
        failedAtMs: Long?,
        nowMs: Long,
        unknownInitiativeSeen: Boolean,
    ): Boolean {
        if (failedAtMs != null && nowMs - failedAtMs < TREE_RETRY_COOLDOWN_MS) return false
        return tree == null || unknownInitiativeSeen
    }

    /** Recomputes eligible old ready_for_review tasks for [initiativeId] from the current task list. */
    fun eligibleForGroup(
        tasks: List<TaskItem>,
        initiativeId: String?,
        now: LocalDateTime = LocalDateTime.now(ZoneOffset.UTC),
    ): List<TaskItem> {
        val targetInit = initiativeId?.takeIf { it.isNotBlank() }
        return tasks.filter { t ->
            TasksLogic.open(t) &&
                t.initiativeId?.takeIf { it.isNotBlank() } == targetInit &&
                t.status == "ready_for_review" &&
                Aging.isOld(t.updatedAt, now)
        }
    }
}

/** Result of moving old ready_for_review tasks in a waiting group to complete. */
data class BulkReviewOutcome(
    val moved: List<TaskItem>,
    val results: List<Applied>,
)

/** Result of undoing a bulk review move. */
data class UndoReviewOutcome(
    val restoredCount: Int,
    val failedCount: Int,
)

/** Bulk mark-reviewed and Undo execution over a patch callback so tests can verify the exact id sets. */
object WaitingBulkReview {
    suspend fun markReviewed(
        tasks: List<TaskItem>,
        patch: suspend (String, JSONObject) -> Unit,
    ): BulkReviewOutcome {
        val eligible = tasks.filter { it.status == "ready_for_review" }
        val gate = Semaphore(BulkEdit.IN_FLIGHT)
        val outcomes = coroutineScope {
            eligible.map { t -> async { gate.withPermit { patchOne(t, patch) } } }.awaitAll()
        }
        val moved = outcomes.filter { it.second == Applied.Changed }.map { it.first }
        return BulkReviewOutcome(moved = moved, results = outcomes.map { it.second })
    }

    private suspend fun patchOne(
        t: TaskItem,
        patch: suspend (String, JSONObject) -> Unit,
    ): Pair<TaskItem, Applied> {
        val body = BulkEdit.patchFor(Bulk.Status("complete"), t) ?: return t to Applied.Already
        return try {
            patch(t.id, body)
            t to Applied.Changed
        } catch (e: HqUnavailable) {
            t to Applied.Failed(t.displayId.ifBlank { t.title }, BulkEdit.failure(e.status))
        }
    }

    /** Formats the snackbar line for [outcome], reporting partial failures alongside moved counts. */
    fun summaryLine(w: Words, outcome: BulkReviewOutcome): String {
        val movedCount = outcome.moved.size
        if (movedCount == 0) return BulkEdit.summary(w, Bulk.Status("complete"), outcome.results)
        val failedCount = outcome.results.count { it is Applied.Failed }
        val base = w.s(R.string.tasks_marked_reviewed, movedCount.toString())
        if (failedCount == 0) return base
        return w.n(R.plurals.tasks_marked_reviewed_partial, failedCount, base, failedCount.toString())
    }

    /** Restores only [moved] back to ready_for_review with expected_status complete; changed tasks are skipped and counted. */
    suspend fun undoReviewed(
        moved: List<TaskItem>,
        patch: suspend (String, JSONObject) -> Unit,
    ): UndoReviewOutcome {
        var restored = 0
        var failed = 0
        for (t in moved) {
            val body = BulkEdit.patchFor(Bulk.Status("ready_for_review"), t.copy(status = "complete")) ?: continue
            try {
                patch(t.id, body)
                restored++
            } catch (e: HqUnavailable) {
                failed++
            }
        }
        return UndoReviewOutcome(restoredCount = restored, failedCount = failed)
    }
}
