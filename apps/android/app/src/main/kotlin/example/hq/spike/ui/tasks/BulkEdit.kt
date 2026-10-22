package example.hq.spike.ui.tasks

import example.hq.spike.data.HqClient.TaskItem
import org.json.JSONObject

/** One change made to every selected task: a status or a priority (null is "None"). */
sealed interface Bulk {
    data class Status(val to: String) : Bulk
    data class Priority(val to: String?) : Bulk
}

/** What happened to one task in a bulk change. */
sealed interface Applied {
    data object Changed : Applied
    data object Already : Applied
    data class Failed(val displayId: String, val why: String) : Applied
}

/** The words and the request body for changing many tasks at once. */
object BulkEdit {
    const val IN_FLIGHT = 4
    val PRIORITY_CHOICES = listOf<Pair<String?, String>>("urgent" to "Urgent", "high" to "High", "normal" to "Normal", "low" to "Low", null to "None")
    val STATUS_CHOICES = listOf("to_do", "in_progress", "blocked", "ready_for_review", "complete")

    /** The PATCH body for [t], or null when it already has that value. A status change carries the status it was read with, so one changed meanwhile is refused. */
    fun patchFor(change: Bulk, t: TaskItem): JSONObject? = when (change) {
        is Bulk.Status -> if (t.status == change.to) null else JSONObject().put("status", change.to).put("expected_status", t.status)
        is Bulk.Priority -> if (t.priority == change.to) null else JSONObject().put("priority", change.to ?: JSONObject.NULL)
    }

    fun failure(status: Int): String = when (status) {
        409 -> "was changed by someone else"
        404 -> "is not there any more"
        400 -> "was not accepted"
        else -> "could not be reached"
    }

    private fun tasks(n: Int) = if (n == 1) "1 task" else "$n tasks"

    private fun priorityWords(p: String?) = PRIORITY_CHOICES.firstOrNull { it.first == p }?.second ?: "None"

    /** "3 tasks moved to Done, 1 already there. 2 could not be updated: FR-12 was changed by someone else". */
    fun summary(change: Bulk, results: List<Applied>): String {
        val changed = results.count { it == Applied.Changed }
        val already = results.count { it == Applied.Already }
        val failed = results.filterIsInstance<Applied.Failed>()
        val to = when (change) {
            is Bulk.Status -> "moved to ${TasksLogic.statusWords(change.to)}"
            is Bulk.Priority -> if (change.to == null) "set to no priority" else "set to ${priorityWords(change.to)}"
        }
        val done = buildList {
            if (changed > 0) add("${tasks(changed)} $to")
            if (already > 0) add(if (changed > 0) "$already already there" else "${tasks(already)} already ${if (change is Bulk.Status) "there" else "that"}")
        }.joinToString(", ")
        val bad = if (failed.isEmpty()) "" else "${failed.size} could not be updated: " + failed.joinToString("; ") { "${it.displayId} ${it.why}" }
        return listOf(done, bad).filter { it.isNotEmpty() }.joinToString(". ")
    }
}
