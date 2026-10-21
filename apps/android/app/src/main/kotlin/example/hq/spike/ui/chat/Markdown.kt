package example.hq.spike.ui.chat

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import example.hq.spike.ui.theme.Mts

private val Inline = Regex("\\*\\*(.+?)\\*\\*|`([^`]+)`")
private val Bullet = Regex("^\\s*[-*] ")

/**
 * The small part of Markdown a first reply needs: headings and bold as bold, inline code in a mono face,
 * list lines with a bullet. Tables, charts, note cards and code blocks come with the rich-block slice.
 */
fun markdown(text: String): AnnotatedString = buildAnnotatedString {
    val lines = text.lines()
    lines.forEachIndexed { i, raw ->
        val heading = raw.startsWith("#")
        val line = Bullet.replace(if (heading) raw.trimStart('#').trimStart() else raw, "• ")
        if (heading) withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { inline(line) } else inline(line)
        if (i < lines.lastIndex) append('\n')
    }
}

private fun AnnotatedString.Builder.inline(line: String) {
    var at = 0
    for (m in Inline.findAll(line)) {
        append(line.substring(at, m.range.first))
        val bold = m.groups[1]?.value
        if (bold != null) {
            withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { append(bold) }
        } else {
            withStyle(SpanStyle(fontFamily = FontFamily.Monospace, background = Mts.Surface200)) { append(m.groups[2]!!.value) }
        }
        at = m.range.last + 1
    }
    append(line.substring(at))
}
