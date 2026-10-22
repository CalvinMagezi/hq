package example.hq.spike.ui.chat.blocks

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import example.hq.spike.ui.theme.Mts

private val MARKS = Regex("\\*\\*(.+?)\\*\\*|`{1,2}([^`\\n]+)`{1,2}|\\[\\[([^\\]|]+)(?:\\|([^\\]]+))?]]|!?\\[([^\\]]+)]\\(([^)]+)\\)")
private val FENCE_LINE = Regex("^\\s*```.*\\n?", RegexOption.MULTILINE)
private val WHITESPACE = Regex("\\s+")
private val OPEN_MARKS = Regex("\\*\\*|`")
private val HEADING = Regex("^#{1,6}\\s+", RegexOption.MULTILINE)

/** Markdown marks taken out of a line that is shown as plain text: a table cell, a search snippet, a notification. */
object Plain {
    fun strip(text: String): String = MARKS.replace(text.replace(FENCE_LINE, "")) { m ->
        m.groups[1]?.value ?: m.groups[2]?.value ?: (m.groups[4]?.value ?: m.groups[3]?.value) ?: m.groups[5]?.value.orEmpty()
    }.replace(HEADING, "").trim()

    /**
     * [strip], on a single line: a snippet whose text had line breaks would otherwise leave a lone "…" on a line of its own.
     * A break between a line that ends in a word and a line that starts a capital is a sentence break (a heading or a list
     * item had no full stop), so it is joined with one; any other break is joined with a space.
     */
    fun oneLine(text: String): String {
        val lines = strip(text).lines().map { it.replace(WHITESPACE, " ").trim() }.filter { it.isNotEmpty() }
        return lines.foldIndexed("") { i, acc, line ->
            val sentenceBreak = i > 0 && acc.last().isLetterOrDigit() && line.first().isUpperCase()
            if (i == 0) line else acc + (if (sentenceBreak) ". " else " ") + line
        }
    }

    /** A line cut short by the server can end inside a mark, so any mark still open is dropped too. */
    fun preview(text: String): String = oneLine(text).replace(OPEN_MARKS, "")

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
