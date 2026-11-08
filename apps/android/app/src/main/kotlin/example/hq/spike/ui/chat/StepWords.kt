package example.hq.spike.ui.chat

import androidx.annotation.StringRes
import example.hq.spike.R
import example.hq.spike.data.ToolStep
import org.json.JSONException
import org.json.JSONObject
import java.net.URI
import java.net.URISyntaxException

/** One line of "What I did": a sentence from the catalogue and, when the step named something, that name. */
data class StepLine(@StringRes val res: Int, val arg: String? = null)

/**
 * The plain words for what a reply did, instead of tool names. The web has the same table in
 * apps/hq-web/src/lib/stepWords.ts; keep the two in step.
 */
object ToolWords {
    private enum class Slot { Query, Site, Note, Title }

    private class Named(@StringRes val with: Int, @StringRes val without: Int, val slot: Slot)

    private val NAMED = mapOf(
        "web_search" to Named(R.string.step_web_search_for, R.string.step_web_search, Slot.Query),
        "web_fetch" to Named(R.string.step_web_fetch_site, R.string.step_web_fetch, Slot.Site),
        "vault_search" to Named(R.string.step_notes_search_for, R.string.step_notes_search, Slot.Query),
        "vault_find" to Named(R.string.step_notes_search_for, R.string.step_notes_search, Slot.Query),
        "vault_read" to Named(R.string.step_note_read_named, R.string.step_note_read, Slot.Note),
        "vault_read_section" to Named(R.string.step_note_read_named, R.string.step_note_read, Slot.Note),
        "vault_outline" to Named(R.string.step_note_read_named, R.string.step_note_read, Slot.Note),
        "vault_write_note" to Named(R.string.step_note_saved_named, R.string.step_note_saved, Slot.Note),
        "vault_append_note" to Named(R.string.step_note_updated_named, R.string.step_note_updated, Slot.Note),
        "vault_patch_section" to Named(R.string.step_note_updated_named, R.string.step_note_updated, Slot.Note),
        "vault_frontmatter_update" to Named(R.string.step_note_updated_named, R.string.step_note_updated, Slot.Note),
        "task_create" to Named(R.string.step_task_added_named, R.string.step_task_added, Slot.Title),
        "task_create_from_note" to Named(R.string.step_task_added_named, R.string.step_task_added, Slot.Title),
    )

    private val PLAIN: Map<String, Int> = buildMap {
        fun all(@StringRes res: Int, vararg names: String) = names.forEach { put(it, res) }
        all(R.string.step_notes_search, "vault_find_similar")
        all(
            R.string.step_notes_looked, "vault_batch_read", "vault_list", "vault_tags", "vault_links", "vault_backlinks", "vault_log",
            "vault_context", "vault_inventory", "context_packet", "folder_list",
        )
        all(R.string.step_note_moved, "vault_move")
        all(R.string.step_note_deleted, "vault_delete")
        all(R.string.step_notes_imported, "vault_import")
        all(R.string.step_document_made, "vault_export", "vault_export_pdf", "convert_from_markdown")
        all(R.string.step_document_read, "convert_to_markdown")
        all(R.string.step_picture_read, "ocr_extract_text")
        all(R.string.step_picture_made, "generate_image")
        all(
            R.string.step_tasks_checked, "task_list", "task_get", "task_next", "task_related", "task_stale", "task_activity", "task_decisions",
            "task_time_report", "task_comment_list", "task_link_list", "initiative_list", "initiative_progress", "space_list",
        )
        all(R.string.step_tasks_added, "task_create_many")
        all(
            R.string.step_tasks_updated, "task_update", "task_update_many", "task_claim", "task_release", "task_restore", "task_link_add",
            "task_link_remove", "initiative_create", "space_create", "space_update",
        )
        all(R.string.step_task_commented, "task_comment_add")
        all(R.string.step_task_removed, "task_delete")
        all(R.string.step_memory_checked, "memory_entity_graph")
        all(R.string.step_memory_saved, "vault_person_record_fact")
        all(R.string.step_chats_searched, "session_search", "history_restore")
        all(R.string.step_earlier_result, "tool_output_read")
        all(R.string.step_helper_asked, "spawn_subagents", "agent_delegate")
        all(R.string.step_helper_started, "harness_session_spawn")
        all(R.string.step_helper_checked, "list_agents")
        all(R.string.step_command_ran, "bash", "run_command")
        all(
            R.string.step_files_looked, "read_file", "list_dir", "find_files", "grep", "document_symbols", "workspace_symbols", "goto_definition",
            "find_references", "hover",
        )
        all(R.string.step_files_changed, "write_file", "edit_file", "file_edit_batch", "rollback_file")
        all(R.string.step_code_looked, "git_status", "git_diff", "git_log", "github_read", "github_clone")
        all(R.string.step_code_saved, "git_commit", "git_pr", "github_write")
        all(R.string.step_google_used, "google_workspace")
        all(R.string.step_owner_notified, "notify_owner")
        all(R.string.step_watch_set, "watch_create")
        all(R.string.step_watch_stopped, "watch_cancel")
        all(R.string.step_watch_checked, "watch_list")
        all(R.string.step_skill_used, "load_skill", "list_skills")
        all(R.string.step_skill_changed, "skill_manage")
        all(R.string.step_plan_made, "todo_write")
        all(R.string.step_progress_shared, "report_progress")
        all(R.string.step_hq_checked, "system_info", "budget_status")
        all(R.string.step_writing_checked, "prose_lint")
    }

    /** Tools no table names still say roughly what they touched. */
    private val PREFIXES = listOf(
        "vault_" to R.string.step_notes_worked,
        "task_" to R.string.step_tasks_worked,
        "initiative_" to R.string.step_tasks_worked,
        "space_" to R.string.step_tasks_worked,
        "harness_session_" to R.string.step_helper_checked,
        "host_" to R.string.step_helper_checked,
        "subagent_run_" to R.string.step_helper_checked,
        "git_" to R.string.step_code_worked,
        "github_" to R.string.step_code_worked,
        "web_" to R.string.step_web_worked,
        "self_update_" to R.string.step_hq_checked,
    )

    /** Long names are cut so a line fits on a phone. */
    private const val MAX_NAME = 60

    /** The plain line for one tool call. */
    fun line(step: ToolStep): StepLine {
        val (name, args) = unwrap(step.name, parse(step.args))
        NAMED[name]?.let { named ->
            val value = pick(named.slot, args)
            return if (value != null) StepLine(named.with, value) else StepLine(named.without)
        }
        PLAIN[name]?.let { return StepLine(it) }
        PREFIXES.firstOrNull { name.startsWith(it.first) }?.let { return StepLine(it.second) }
        return StepLine(R.string.step_used, name.replace('_', ' ').trim().ifEmpty { name })
    }

    /** Every step as a line, with a run of the same line said once ("Checked your tasks" three times reads once). */
    fun lines(steps: List<ToolStep>): List<StepLine> =
        steps.map(::line).fold(mutableListOf<StepLine>()) { out, l -> out.apply { if (lastOrNull() != l) add(l) } }

    private fun parse(args: String?): JSONObject? = try {
        args?.takeIf { it.isNotBlank() }?.let { JSONObject(it) }
    } catch (e: JSONException) {
        null
    }

    /** The gateway's `hq_call` carries the real tool in its arguments. */
    private fun unwrap(name: String, args: JSONObject?): Pair<String, JSONObject?> {
        if (name != "hq_call" || args == null) return name to args
        val inner = raw(args, "tool") ?: raw(args, "name") ?: return name to args
        val innerArgs = args.optJSONObject("args") ?: args.optJSONObject("arguments")
        return inner to innerArgs
    }

    private fun pick(slot: Slot, args: JSONObject?): String? = when (slot) {
        Slot.Query -> raw(args, "query")
        Slot.Title -> raw(args, "title")
        Slot.Note -> raw(args, "path")?.split('/')?.lastOrNull { it.isNotBlank() }?.removeSuffix(".md")
        Slot.Site -> raw(args, "url")?.let(::host)
    }?.let(::short)

    /** "https://www.example.com/a/b" reads as "example.com". */
    private fun host(url: String): String? = try {
        URI(url.trim()).host?.removePrefix("www.")
    } catch (e: URISyntaxException) {
        null
    }

    private fun raw(args: JSONObject?, key: String): String? = (args?.opt(key) as? String)?.takeIf { it.isNotBlank() }

    private fun short(s: String): String? {
        val one = s.replace(Regex("\\s+"), " ").trim()
        if (one.isEmpty()) return null
        return if (one.length > MAX_NAME) one.take(MAX_NAME - 1).trimEnd() + "…" else one
    }
}
