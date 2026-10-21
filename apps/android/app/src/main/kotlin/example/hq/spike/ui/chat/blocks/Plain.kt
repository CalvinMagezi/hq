package example.hq.spike.ui.chat.blocks

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import example.hq.spike.ui.theme.Mts

private val MARKS = Regex("\\*\\*(.+?)\\*\\*|`([^`\\n]+)`|\\[\\[([^\\]|]+)(?:\\|([^\\]]+))?]]|!?\\[([^\\]]+)]\\(([^)]+)\\)")
private val FENCE_LINE = Regex("^\\s*```.*\\n?", RegexOption.MULTILINE)
private val HEADING = Regex("^#{1,6}\\s+", RegexOption.MULTILINE)

/** Markdown marks taken out of a line that is shown as plain text: a table cell, a search snippet, a notification. */
object Plain {
    fun strip(text: String): String = MARKS.replace(text.replace(FENCE_LINE, "")) { m ->
        m.groups[1]?.value ?: m.groups[2]?.value ?: (m.groups[4]?.value ?: m.groups[3]?.value) ?: m.groups[5]?.value.orEmpty()
    }.replace(HEADING, "").trim()

    /** A table cell: bold and code keep their look, links and wikilinks show only their words. */
    fun cell(text: String): AnnotatedString = buildAnnotatedString {
        var at = 0
        for (m in MARKS.findAll(text)) {
            append(text.substring(at, m.range.first))
            at = m.range.last + 1
            val bold = m.groups[1]?.value
            val code = m.groups[2]?.value
            when {
                bold != null -> withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { append(bold) }
                code != null -> withStyle(SpanStyle(fontFamily = FontFamily.Monospace, background = Mts.Surface200)) { append(code) }
                else -> append(m.groups[4]?.value ?: m.groups[3]?.value ?: m.groups[5]?.value.orEmpty())
            }
        }
        append(text.substring(at))
    }
}
