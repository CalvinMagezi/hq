package example.hq.spike.data

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject

// The task calls of [HqClient]: the tree, briefings, lists, edits and comments, one extension each.

/** Where tasks live: spaces hold folders and initiatives, a folder holds initiatives, and every task belongs to one initiative. */
suspend fun HqClient.taskTree(): example.hq.spike.ui.tasks.TaskTree {
    val spaces = send(authed("/api/spaces").build()) { t -> JSONObject(t).optJSONArray("spaces").objects().map { example.hq.spike.ui.tasks.TaskSpace(it.getString("id"), it.optString("name", it.getString("id"))) } }
    val folders = send(authed("/api/folders").build()) { t -> JSONObject(t).optJSONArray("folders").objects().map { example.hq.spike.ui.tasks.TaskFolder(it.getString("id"), it.optString("space_id"), it.optString("name", it.getString("id"))) } }
    val initiatives = send(authed("/api/initiatives").build()) { t ->
        JSONObject(t).optJSONArray("initiatives").objects().map { example.hq.spike.ui.tasks.TaskInitiative(it.getString("id"), it.optString("space_id"), it.optStringOrNull("folder_id"), it.optString("name", it.getString("id"))) }
    }
    return example.hq.spike.ui.tasks.TaskTree(spaces, folders, initiatives)
}

/** The briefing for one initiative, as HQ generates it now. A 404 is an edition with no agents (Lite): the caller says so. */
suspend fun HqClient.initiativeBriefing(id: String): example.hq.spike.ui.tasks.Briefing = send(authed("/api/initiatives/${java.net.URLEncoder.encode(id, "UTF-8").replace("+", "%20")}/activity").build()) { text ->
    val o = JSONObject(text)
    val counts = { name: String -> o.optJSONObject(name)?.let { c -> c.keys().asSequence().map { it to c.optInt(it) }.toList() }.orEmpty() }
    example.hq.spike.ui.tasks.Briefing(
        name = o.optJSONObject("initiative")?.optString("name").orEmpty(),
        tasksShown = o.optInt("tasks_shown"),
        statusCounts = counts("status_counts"),
        awaiting = o.optJSONArray("awaiting_review").objects().map { example.hq.spike.ui.tasks.BriefTask(it.getString("id"), it.optString("display_id"), it.optString("title")) },
        blocked = o.optJSONArray("blocked").objects().map { example.hq.spike.ui.tasks.BlockedTask(it.getString("id"), it.optString("display_id"), it.optString("title"), it.optStringOrNull("reason"), it.optStringOrNull("waiting_on")) },
        contributors = o.optJSONArray("contributors").objects().map { example.hq.spike.ui.tasks.Contributor(it.optString("actor"), it.optInt("notes"), it.optInt("leases"), it.optStringOrNull("last_active")) },
        reviewCounts = counts("review_counts"),
        gapsTotal = o.optInt("gaps_total"),
        notes = o.optJSONArray("recent_notes").objects().map {
            example.hq.spike.ui.tasks.BriefNote(
                it.optString("path"), it.optString("task_display_id"),
                example.hq.spike.ui.tasks.BriefingWords.author(it.optString("attribution"), it.optStringOrNull("actor"), it.optStringOrNull("source_kind")),
                it.optStringOrNull("written_at"), it.optStringOrNull("review_state"),
            )
        },
        freshness = o.optJSONObject("freshness")?.let { f -> f.keys().asSequence().map { it to f.optStringOrNull(it) }.toList() }.orEmpty(),
    )
}

/** The tasks in progress, newest first, with their count: what the Tasks widget shows. */
suspend fun HqClient.inProgress(limit: Int): Pair<List<example.hq.spike.capture.TaskLine>, Int> = send(authed("/api/tasks?status=in_progress&top_level_only=true&limit=$limit").build()) { text ->
    val o = JSONObject(text)
    val rows = o.optJSONArray("tasks").objects().map { example.hq.spike.capture.TaskLine(it.getString("id"), example.hq.spike.capture.Widgets.numberOf(it.optString("display_id", "")), it.optString("title", "")) }
    rows to o.optInt("total", rows.size)
}

/** The helpers working on a task now (a lease that has not ended). */
suspend fun HqClient.liveWorkSessions(): List<HqClient.WorkSession> = send(authed("/api/work-sessions?days=1").build()) { text ->
    JSONObject(text).optJSONArray("work_sessions").objects().filter { it.isNull("ended_at") || !it.has("ended_at") }
        .map { HqClient.WorkSession(it.optString("task_id"), it.optString("actor"), it.optString("harness"), it.optString("cwd")) }
}

/** Active tasks, or the sub-tasks of one. The lists group and count them on the phone. */
suspend fun HqClient.tasks(parentId: String? = null, archived: Boolean = false): List<HqClient.TaskItem> {
    // Pages are read one after another while tasks change, so a row can come twice (or be missed); each id is kept once.
    val all = LinkedHashMap<String, HqClient.TaskItem>()
    var offset = 0
    while (all.size < HqClient.TASK_CAP) {
        val query = "limit=$HqClient.TASK_PAGE&offset=$offset" + (parentId?.let { "&parent_task_id=$it" } ?: "&top_level_only=true") + (if (archived) "&archived=true" else "")
        val (page, more) = send(authed("/api/tasks?$query").build()) { text ->
            val o = JSONObject(text)
            o.optJSONArray("tasks").objects().map { taskOf(it) } to o.optBoolean("has_more", false)
        }
        val before = all.size
        page.forEach { all[it.id] = it }
        offset += page.size
        if (!more || page.isEmpty() || all.size == before) break
    }
    return all.values.toList()
}

/** Changes some fields of a task; a field left out is untouched, a JSON null clears it. 409 means it changed meanwhile. */
suspend fun HqClient.patchTask(id: String, change: JSONObject): HqClient.TaskItem =
    send(authed("/api/tasks/${java.net.URLEncoder.encode(id, "UTF-8")}").patch(json(change)).build()) { taskOf(JSONObject(it)) }

suspend fun HqClient.createTask(body: JSONObject): HqClient.TaskItem = send(authed("/api/tasks").post(json(body)).build()) { taskOf(JSONObject(it)) }

/** Adds a task with a repeat key; true when HQ already had one with that key and so added nothing. */
suspend fun HqClient.createTaskOnce(body: JSONObject): Boolean = createTaskNamed(body).first

/** Like [createTaskOnce] and also the id of the task, so it can be taken back; null when HQ did not say. */
suspend fun HqClient.createTaskNamed(body: JSONObject): Pair<Boolean, String?> = send(authed("/api/tasks").post(json(body)).build()) {
    val o = JSONObject(it)
    o.optBoolean("deduplicated") to o.optString("id", "").ifBlank { null }
}

suspend fun HqClient.taskComments(id: String): List<HqClient.TaskComment> =
    send(authed("/api/tasks/${java.net.URLEncoder.encode(id, "UTF-8")}/comments").build()) { text ->
        JSONObject(text).optJSONArray("comments").objects().map { HqClient.TaskComment(it.optString("id"), it.optString("author", ""), it.optString("body", ""), it.optString("created_at", "")) }
    }

suspend fun HqClient.addTaskComment(id: String, body: String) {
    send(authed("/api/tasks/${java.net.URLEncoder.encode(id, "UTF-8")}/comments").post(json(JSONObject().put("body", body).put("author", "operator"))).build()) { }
}

suspend fun HqClient.archiveTask(id: String) { send(authed("/api/tasks/${java.net.URLEncoder.encode(id, "UTF-8")}").delete().build()) { } }

suspend fun HqClient.restoreTask(id: String) { send(authed("/api/tasks/${java.net.URLEncoder.encode(id, "UTF-8")}/restore").post(json(JSONObject())).build()) { } }
