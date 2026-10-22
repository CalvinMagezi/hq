package example.hq.spike.ui.notes

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue

/** The format bar's edits to a Markdown text and the selection in it. Each returns the new text with the cursor where the writer expects it. */
object MarkdownEditing {
    /** Makes the current line a heading, or cycles it: none, ##, ###, none. */
    fun heading(v: TextFieldValue): TextFieldValue {
        val (start, end) = lineBounds(v.text, v.selection.min)
        val line = v.text.substring(start, end)
        val body = line.trimStart('#').trimStart()
        val level = line.length - line.trimStart('#').length
        val next = when (level) { 0 -> "## "; 2 -> "### "; else -> "" }
        return replace(v, start, end, next + body, cursorAtEnd = true)
    }

    /** Wraps the selection in [mark] (bold `**`, italic `*`), or removes the mark when it is already there. With no selection it places the cursor between two marks. */
    fun wrap(v: TextFieldValue, mark: String): TextFieldValue {
        val s = v.selection.min
        val e = v.selection.max
        val before = v.text.substring(0, s)
        val picked = v.text.substring(s, e)
        val after = v.text.substring(e)
        if (picked.length > 2 * mark.length && picked.startsWith(mark) && picked.endsWith(mark)) {
            val plain = picked.removePrefix(mark).removeSuffix(mark)
            return TextFieldValue(before + plain + after, TextRange(s, s + plain.length))
        }
        // The marks sit just outside the selection, as they do right after the selection was wrapped.
        if (picked.isNotEmpty() && before.endsWith(mark) && after.startsWith(mark)) {
            return TextFieldValue(before.dropLast(mark.length) + picked + after.drop(mark.length), TextRange(s - mark.length, e - mark.length))
        }
        val text = before + mark + picked + mark + after
        return TextFieldValue(text, if (picked.isEmpty()) TextRange(s + mark.length) else TextRange(s + mark.length, e + mark.length))
    }

    /** Puts [prefix] ("- ", "- [ ] ") at the start of every line the selection touches, or takes it off when they all have it. */
    fun prefixLines(v: TextFieldValue, prefix: String): TextFieldValue {
        val (start, _) = lineBounds(v.text, v.selection.min)
        val (_, end) = lineBounds(v.text, v.selection.max)
        val lines = v.text.substring(start, end).split("\n")
        val all = lines.all { it.startsWith(prefix) }
        val changed = lines.joinToString("\n") { if (all) it.removePrefix(prefix) else if (it.startsWith(prefix)) it else prefix + it }
        return replace(v, start, end, changed, cursorAtEnd = true)
    }

    /** Links to another note: the selection goes between `[[` and `]]`, or the cursor sits between them when nothing is selected. */
    fun link(v: TextFieldValue): TextFieldValue {
        val s = v.selection.min
        val e = v.selection.max
        val text = v.text.substring(0, s) + "[[" + v.text.substring(s, e) + "]]" + v.text.substring(e)
        return TextFieldValue(text, if (s == e) TextRange(s + 2) else TextRange(e + 4))
    }

    private fun lineBounds(text: String, at: Int): Pair<Int, Int> {
        val start = text.lastIndexOf('\n', at - 1) + 1
        val end = text.indexOf('\n', at).let { if (it < 0) text.length else it }
        return start to end
    }

    private fun replace(v: TextFieldValue, start: Int, end: Int, with: String, cursorAtEnd: Boolean): TextFieldValue {
        val text = v.text.substring(0, start) + with + v.text.substring(end)
        return TextFieldValue(text, TextRange(if (cursorAtEnd) start + with.length else v.selection.min))
    }
}
