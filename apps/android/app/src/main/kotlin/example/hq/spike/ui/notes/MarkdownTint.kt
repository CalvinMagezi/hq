package example.hq.spike.ui.notes

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.OffsetMapping
import androidx.compose.ui.text.input.TransformedText
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.sp
import example.hq.spike.ui.theme.Mts

enum class TintKind { Mark, Heading, Wiki }

data class TintSpan(val start: Int, val end: Int, val kind: TintKind)

/** Which parts of a Markdown text the editor colours: the marks grey, heading text large, `[[wiki links]]` blue. It only reads the text. */
object MarkdownTint {
    /** A longer text is left plain so typing in a big note stays quick. */
    const val MAX_CHARS = 200_000

    /** A longer line is left plain: the patterns slow down on a line with thousands of unclosed marks. */
    const val MAX_LINE = 2_000
    private const val HEADING_SIZE = 18

    private val HEADING = Regex("^(#{1,6} )")
    private val LIST = Regex("^\\s*[-*+] (?:\\[[ xX]] )?")
    private val BOLD = Regex("\\*\\*(?=\\S)(.+?)(?<=\\S)\\*\\*")
    private val ITALIC = Regex("(?<![*\\w])\\*(?=[^\\s*])(.+?)(?<=[^\\s*])\\*(?![*\\w])")
    private val WIKI = Regex("\\[\\[[^\\]\\n]+]]")

    fun spans(text: String): List<TintSpan> {
        val out = ArrayList<TintSpan>()
        var lineStart = 0
        var fenced = false
        for (line in text.split('\n')) {
            if (line.trimStart().startsWith("```")) fenced = !fenced
            else if (!fenced && line.length <= MAX_LINE) tintLine(line, lineStart, out)
            lineStart += line.length + 1
        }
        return out
    }

    private fun tintLine(line: String, at: Int, out: MutableList<TintSpan>) {
        var from = 0
        HEADING.find(line)?.let { h ->
            out += TintSpan(at, at + h.value.length, TintKind.Mark)
            if (line.length > h.value.length) out += TintSpan(at + h.value.length, at + line.length, TintKind.Heading)
            from = h.value.length
        } ?: LIST.find(line)?.let { l ->
            out += TintSpan(at, at + l.value.length, TintKind.Mark)
            from = l.value.length
        }
        val rest = line.substring(from)
        val masked = rest.toCharArray()
        for (b in BOLD.findAll(rest)) {
            out += TintSpan(at + from + b.range.first, at + from + b.range.first + 2, TintKind.Mark)
            out += TintSpan(at + from + b.range.last - 1, at + from + b.range.last + 1, TintKind.Mark)
            for (i in b.range) masked[i] = ' '
        }
        for (i in ITALIC.findAll(String(masked))) {
            out += TintSpan(at + from + i.range.first, at + from + i.range.first + 1, TintKind.Mark)
            out += TintSpan(at + from + i.range.last, at + from + i.range.last + 1, TintKind.Mark)
        }
        for (w in WIKI.findAll(rest)) out += TintSpan(at + from + w.range.first, at + from + w.range.last + 1, TintKind.Wiki)
    }
}

/** The editor's tint as a style over the text, so the text and the cursor positions are the ones typed. */
object MarkdownTransformation : VisualTransformation {
    private val Mark = SpanStyle(color = Mts.Line, fontWeight = FontWeight.Normal)
    private val Heading = SpanStyle(color = Mts.Ink, fontWeight = FontWeight.Bold, fontSize = 18.sp)
    private val Wiki = SpanStyle(color = Mts.BrandBlue, fontWeight = FontWeight.Normal)

    /** The last text tinted and its result: a cursor move runs the transformation again over the same text. */
    @Volatile private var last: Pair<String, TransformedText>? = null

    override fun filter(text: AnnotatedString): TransformedText {
        if (text.length > MarkdownTint.MAX_CHARS) return TransformedText(text, OffsetMapping.Identity)
        last?.let { (was, result) -> if (was == text.text) return result }
        val tinted = buildAnnotatedString {
            append(text.text)
            for (s in MarkdownTint.spans(text.text)) {
                addStyle(when (s.kind) { TintKind.Mark -> Mark; TintKind.Heading -> Heading; TintKind.Wiki -> Wiki }, s.start, s.end)
            }
        }
        return TransformedText(tinted, OffsetMapping.Identity).also { last = text.text to it }
    }
}
