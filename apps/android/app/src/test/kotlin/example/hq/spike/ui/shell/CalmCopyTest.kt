package example.hq.spike.ui.shell

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Calm UI, acceptance 1: snackbar and pill/chip copy stays short enough to read at a glance.
 * Discovers snackbar keys by scanning `src/main/kotlin` and combines them with keys passed
 * through variables or helper functions, then checks `apps/strings/en.yaml` directly.
 */
class CalmCopyTest {
    /** Keys passed to snackbars via variables, helpers, or `Line(...)` wrappers that line scanning does not see directly. */
    private val indirectSnackbarKeys = listOf(
        "fail_undo_too_late",
        "dl_set_forgot",
        "dl_set_removed",
        "helpers_allowed_once",
        "helpers_not_now_waits",
        "helpers_follow_gone",
        "helpers_follow_timeout",
        "helpers_open_chat_failed",
        "vault_copied_path",
        "chat_copied",
        "notes_too_long_share",
        "tasks_changed_meanwhile",
        "tasks_not_accepted",
        "tasks_task_gone",
        "tasks_marked_reviewed",
        "tasks_marked_reviewed_partial",
        "search_recent_cleared",
        "chat_send_retry",
        "chat_offer_gone",
        "inbox_fail_no_reply",
        "phone_settings_key_saved",
        "settings_saved_restarted",
        "settings_set_outside",
        "upd_updated_snack",
        "upd_back_allowed",
        "hd_conn_toast",
        "helpers_allowed_toast",
        "helpers_declined_toast",
    )

    private val pillKeys = listOf(
        "inbox_chip_hq",
        "tasks_chip_active",
        "task_stuck",
        "tasks_status_ready",
        "tasks_status_complete",
        "tasks_chip_archived",
        "tasks_filtered",
        "chat_chip_note_1",
        "chat_chip_note_2",
        "chat_chip_note_3",
        "chat_chip_task_1",
        "chat_chip_task_2",
        "chat_chip_task_3",
        "chat_chip_search_1",
        "chat_chip_search_2",
        "chat_chip_search_3",
        "chat_chip_neutral_1",
        "chat_chip_neutral_2",
        "chat_chip_neutral_3",
    )

    private fun mainKotlinDir(): File {
        var dir: File? = File(".").canonicalFile
        while (dir != null) {
            val candidate = File(dir, "src/main/kotlin")
            if (candidate.isDirectory) return candidate
            val fromRoot = File(dir, "apps/android/app/src/main/kotlin")
            if (fromRoot.isDirectory) return fromRoot
            dir = dir.parentFile
        }
        error("Could not locate src/main/kotlin from ${File(".").canonicalPath}")
    }

    private fun catalogueFile(): File {
        var dir: File? = File(".").canonicalFile
        while (dir != null) {
            val candidate = File(dir, "apps/strings/en.yaml")
            if (candidate.isFile) return candidate
            dir = dir.parentFile
        }
        error("Could not locate apps/strings/en.yaml from ${File(".").canonicalPath}")
    }

    private fun discoverSnackbarKeys(): Set<String> {
        val root = mainKotlinDir()
        val ktFiles = root.walkTopDown().filter { it.isFile && it.extension == "kt" }.toList()
        assertTrue("Expected Kotlin source files under ${root.path}", ktFiles.isNotEmpty())

        // Files whose `Notice(...)` call is the inline card composable in `ui/settings/`, not a snackbar.
        val inlineNoticeFiles = setOf("SettingsScreens.kt", "AssistantScreens.kt", "CalendarScreens.kt", "ContactsScreens.kt")
        val callPattern = Regex("""(onNotice\(|say\(|SnackbarRow\(|Toast\.makeText\(|UndoSnackbar\(|InboxLine\(|TaskLine\()""")
        val noticePattern = Regex("""\bNotice\(""")
        val lineSetterPattern = Regex("""\.copy\([^)]*\b(line|undoLine)\s*=""")
        val stringRefPattern = Regex("""R\.string\.([a-z][a-z0-9_]*)""")

        val found = linkedSetOf<String>()
        for (file in ktFiles) {
            val lines = file.readLines()
            val allowNotice = file.name !in inlineNoticeFiles
            for (i in lines.indices) {
                val line = lines[i]
                val isCall = callPattern.containsMatchIn(line) || (allowNotice && noticePattern.containsMatchIn(line))
                val isSetter = lineSetterPattern.containsMatchIn(line)
                if (!isCall && !isSetter) continue
                val window = if (isCall) lines.subList(i, minOf(i + 3, lines.size)).joinToString(" ") else line
                stringRefPattern.findAll(window).forEach { found.add(it.groupValues[1]) }
            }
        }
        return found
    }

    private fun allSnackbarKeys(): Set<String> = discoverSnackbarKeys() + indirectSnackbarKeys

    private fun parseCatalogue(): Map<String, List<String>> {
        val entries = linkedMapOf<String, MutableList<String>>()
        var currentPlural: String? = null
        for (rawLine in catalogueFile().readLines()) {
            val line = rawLine.trimEnd()
            if (line.isBlank() || line.trimStart().startsWith("#")) continue
            val topMatch = Regex("^([a-z][a-z0-9_]*):\\s*(.*)$").matchEntire(line)
            if (topMatch != null) {
                val key = topMatch.groupValues[1]
                val rest = topMatch.groupValues[2].trim()
                if (rest.isEmpty()) {
                    currentPlural = key
                    entries.getOrPut(key) { mutableListOf() }
                } else {
                    currentPlural = null
                    entries[key] = mutableListOf(unquote(rest))
                }
                continue
            }
            val subMatch = Regex("^\\s+[a-z]+:\\s*(.+)$").matchEntire(line)
            if (subMatch != null && currentPlural != null) {
                entries.getOrPut(currentPlural) { mutableListOf() }.add(unquote(subMatch.groupValues[1].trim()))
            }
        }
        return entries
    }

    private fun unquote(raw: String): String {
        if (raw.length >= 2 && raw.startsWith('"') && raw.endsWith('"')) {
            return raw.substring(1, raw.length - 1)
                .replace("\\n", "\n")
                .replace("\\\"", "\"")
                .replace("\\\\", "\\")
        }
        return raw
    }

    private fun fillPlaceholders(text: String, value: String = "1234567890"): String =
        text.replace(Regex("\\{[a-z][a-z0-9_]*\\}"), value)

    private fun sentenceCount(text: String): Int =
        text.trim().split(Regex("[.!?]+")).count { it.trim().isNotEmpty() }

    @Test
    fun snackbarStringsAreOneLineOneSentenceAndAtMost60Chars() {
        val cat = parseCatalogue()
        val keys = allSnackbarKeys()
        assertTrue("Expected discovered snackbar keys", keys.isNotEmpty())
        val problems = buildList {
            for (key in keys) {
                val values = cat[key]
                if (values.isNullOrEmpty()) {
                    add("Missing snackbar key: $key")
                    continue
                }
                for (value in values) {
                    val filled = fillPlaceholders(value)
                    if (filled.contains('\n') || filled.contains('\r')) {
                        add("$key contains a line break: \"$value\"")
                    }
                    if (filled.length > SNACKBAR_MAX_CHARS) {
                        add("$key is ${filled.length} chars (max $SNACKBAR_MAX_CHARS): \"$value\"")
                    }
                    val sentences = sentenceCount(filled)
                    if (sentences > 1) {
                        add("$key has $sentences sentences (max 1): \"$value\"")
                    }
                }
            }
        }
        assertTrue(problems.joinToString("\n"), problems.isEmpty())
    }

    @Test
    fun refusedCaptureToastTemplateStaysOneSentenceWithFortyCharReason() {
        val cat = parseCatalogue()
        val template = cat["capture_refused_toast"]?.singleOrNull()
            ?: error("capture_refused_toast missing from catalogue")
        val filled = fillPlaceholders(template, "x".repeat(40))
        assertEquals(1, sentenceCount(filled))
    }

    @Test
    fun pillAndChipStringsAreAtMost24Chars() {
        val cat = parseCatalogue()
        val problems = buildList {
            for (key in pillKeys) {
                val values = cat[key]
                if (values.isNullOrEmpty()) {
                    add("Missing pill key: $key")
                    continue
                }
                for (value in values) {
                    val filled = fillPlaceholders(value)
                    if (filled.length > PILL_MAX_CHARS) {
                        add("$key is ${filled.length} chars (max $PILL_MAX_CHARS): \"$value\"")
                    }
                }
            }
        }
        assertTrue(problems.joinToString("\n"), problems.isEmpty())
    }

    @Test
    fun everySnackOrToastKeyInCatalogueIsCovered() {
        val cat = parseCatalogue()
        val covered = allSnackbarKeys()
        val missing = cat.keys.filter { (it.endsWith("_snack") || it.endsWith("_toast")) && it !in covered }
        assertTrue("Keys ending in _snack or _toast missing from CalmCopyTest: $missing", missing.isEmpty())
    }

    private companion object {
        const val SNACKBAR_MAX_CHARS = 60
        const val PILL_MAX_CHARS = 24
    }
}
