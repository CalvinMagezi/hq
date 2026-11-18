package example.hq.spike.ui.notes

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue

/** The line starts the toolbar sets. Only one applies to a line at a time. */
sealed interface LineStyle {
    data class Heading(val level: Int) : LineStyle
    data object Bullet : LineStyle
    data object Numbered : LineStyle
    data object Quote : LineStyle
}

private const val BOLD = "**"
private const val ITALIC = "*"
private const val CODE = "`"
private const val LINK_URL = "https://"
private const val LINK_PLACEHOLDER = "text"
private val LINE_START = Regex("^(#{1,6} |[-*+] |\\d+\\. |> )")

private fun prefixOf(style: LineStyle, index: Int): String = when (style) {
    is LineStyle.Heading -> "#".repeat(style.level) + " "
    LineStyle.Bullet -> "- "
    LineStyle.Numbered -> "${index + 1}. "
    LineStyle.Quote -> "> "
}

private fun TextFieldValue.edit(start: Int, end: Int, with: String, selection: TextRange): TextFieldValue =
    TextFieldValue(text.replaceRange(start, end, with), selection)

/**
 * Puts [marker] around the selection (bold, italic, code), or takes it off when the selection is already inside it. With nothing
 * selected it adds an empty pair and leaves the cursor between.
 */
fun TextFieldValue.wrap(marker: String): TextFieldValue {
    val start = selection.min
    val end = selection.max
    val before = text.substring(0, start)
    val after = text.substring(end)
    // A single star is also the end of a bold marker, so it only counts as italic on its own.
    val wrapped = before.endsWith(marker) && after.startsWith(marker) && !(marker == ITALIC && before.endsWith(BOLD))
    if (wrapped) {
        return TextFieldValue(
            before.dropLast(marker.length) + text.substring(start, end) + after.drop(marker.length),
            TextRange(start - marker.length, end - marker.length),
        )
    }
    return edit(start, end, marker + text.substring(start, end) + marker, TextRange(start + marker.length, end + marker.length))
}

fun TextFieldValue.bold() = wrap(BOLD)

fun TextFieldValue.italic() = wrap(ITALIC)

fun TextFieldValue.code() = wrap(CODE)

/**
 * Makes the selection a link: its text stays, and the address `https://` is left selected for typing over. With nothing selected
 * it adds a link with placeholder text.
 */
fun TextFieldValue.link(): TextFieldValue {
    val start = selection.min
    val end = selection.max
    val label = text.substring(start, end).ifEmpty { LINK_PLACEHOLDER }
    val urlStart = start + label.length + 3
    return edit(start, end, "[$label]($LINK_URL)", TextRange(urlStart, urlStart + LINK_URL.length))
}

/**
 * Sets [style] on every line the selection touches, or takes it off when they all have it already. A line that has another
 * style swaps to this one.
 */
fun TextFieldValue.toggleLines(style: LineStyle): TextFieldValue {
    val first = text.lastIndexOf('\n', selection.min - 1) + 1
    val last = text.indexOf('\n', selection.max).let { if (it < 0) text.length else it }
    val lines = text.substring(first, last).split('\n')
    val allHave = lines.withIndex().all { (i, line) -> line.startsWith(prefixOf(style, i)) }
    val changed = lines.mapIndexed { i, line ->
        val bare = line.replaceFirst(LINE_START, "")
        if (allHave) bare else prefixOf(style, i) + bare
    }
    val body = changed.joinToString("\n")
    return TextFieldValue(text.replaceRange(first, last, body), TextRange(first, first + body.length))
}
