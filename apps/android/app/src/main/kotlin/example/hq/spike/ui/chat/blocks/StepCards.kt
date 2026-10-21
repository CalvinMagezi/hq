package example.hq.spike.ui.chat.blocks

import example.hq.spike.data.ToolStep
import java.time.LocalDate
import java.time.format.DateTimeFormatter
import java.time.format.DateTimeParseException
import java.util.Locale
import org.json.JSONException
import org.json.JSONObject

data class NoteCard(val path: String, val title: String, val trail: String)

data class TaskCard(val title: String, val meta: String)

/**
 * What a finished reply made or read, found in its tool steps: notes it saved become cards, notes it only read become
 * chips, tasks it added become cards. Markdown cannot carry these, so the steps are the source.
 */
data class StepCards(val written: List<NoteCard>, val read: List<String>, val tasks: List<TaskCard>) {
    val isEmpty: Boolean get() = written.isEmpty() && read.isEmpty() && tasks.isEmpty()

    companion object {
        val None = StepCards(emptyList(), emptyList(), emptyList())

        fun of(steps: List<ToolStep>, today: LocalDate = LocalDate.now()): StepCards {
            val written = steps.filter { it.name == WRITE_NOTE }.mapNotNull { step ->
                val path = field(step.args, "path")?.let(NoteRefs::sanitize) ?: return@mapNotNull null
                NoteCard(path, field(step.args, "title") ?: NoteRefs.title(path), NoteRefs.folderTrail(path))
            }.distinctBy { it.path }
            val read = steps.filter { it.name == READ_NOTE }.mapNotNull { field(it.args, "path")?.let(NoteRefs::sanitize) }
                .distinct().filter { path -> written.none { it.path == path } }
            val tasks = steps.filter { it.name == CREATE_TASK }.mapNotNull { step ->
                val title = field(step.args, "title") ?: return@mapNotNull null
                TaskCard(title, taskMeta(field(step.args, "due_date"), field(step.args, "priority"), today))
            }
            return StepCards(written, read, tasks)
        }

        private fun taskMeta(due: String?, priority: String?, today: LocalDate): String {
            val parts = listOfNotNull(due?.let { dueWords(it, today) }, priority?.let(::priorityWords))
            return parts.joinToString(" · ").ifEmpty { "In your tasks" }
        }

        private fun dueWords(iso: String, today: LocalDate): String? {
            val day = try { LocalDate.parse(iso) } catch (e: DateTimeParseException) { return null }
            return when (day) {
                today -> "Due today"
                today.plusDays(1) -> "Due tomorrow"
                else -> "Due " + day.format(DateTimeFormatter.ofPattern("EEE d MMM", Locale.getDefault()))
            }
        }

        private fun priorityWords(p: String): String? = when (p.lowercase()) {
            "urgent" -> "Urgent"
            "high" -> "High"
            "low" -> "Low"
            else -> null
        }

        private fun field(args: String?, key: String): String? = try {
            args?.let { JSONObject(it).optString(key, "").trim().ifEmpty { null } }
        } catch (e: JSONException) {
            null
        }

        private const val WRITE_NOTE = "vault_write_note"
        private const val READ_NOTE = "vault_read"
        private const val CREATE_TASK = "task_create"
    }
}
