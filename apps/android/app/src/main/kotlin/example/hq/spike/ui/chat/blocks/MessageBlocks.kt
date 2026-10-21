package example.hq.spike.ui.chat.blocks

enum class Align { Start, End, Center }

/** One block of a reply, in the order it appears. */
sealed interface Block {
    data class Paragraph(val text: String) : Block
    data class Heading(val level: Int, val text: String) : Block
    data class Quote(val text: String) : Block
    data class Bullets(val ordered: Boolean, val items: List<String>) : Block
    data class Checklist(val items: List<Pair<Boolean, String>>) : Block
    data class Code(val lang: String, val text: String, val closed: Boolean) : Block
    data class ChartBlock(val result: ChartParse, val source: String, val closed: Boolean) : Block
    data class Table(val headers: List<String>, val aligns: List<Align>, val rows: List<List<String>>) : Block
    data class Image(val alt: String) : Block
}

/**
 * Splits a reply into blocks the way marked does for the web app: fenced code (a `chart` fence or a Mermaid pie
 * becomes a chart), headings, quotes, lists, checklists, GFM tables, pictures, paragraphs. A fence that has not
 * closed yet (the reply is still streaming) is returned with `closed = false` so it can say "Drawing the chart".
 */
fun parseBlocks(text: String): List<Block> {
    val lines = text.lines()
    val out = mutableListOf<Block>()
    var i = 0
    while (i < lines.size) {
        val line = lines[i]
        val fence = FENCE.matchEntire(line)
        when {
            line.isBlank() -> i++
            fence != null -> i = fenced(lines, i, fence.groupValues[1], out)
            HEADING.matches(line) -> { val m = HEADING.matchEntire(line)!!; out += Block.Heading(m.groupValues[1].length, m.groupValues[2].trim()); i++ }
            HRULE.matches(line) -> i++
            IMAGE.matches(line) -> { out += Block.Image(IMAGE.matchEntire(line)!!.groupValues[1]); i++ }
            line.contains('|') && i + 1 < lines.size && TABLE_RULE.matches(lines[i + 1]) -> i = table(lines, i, out)
            QUOTE.containsMatchIn(line) -> i = quote(lines, i, out)
            CHECK.matches(line) -> i = checklist(lines, i, out)
            BULLET.matches(line) || ORDERED.matches(line) -> i = list(lines, i, out)
            else -> i = paragraph(lines, i, out)
        }
    }
    return out
}

private val FENCE = Regex("^\\s*```\\s*([\\w+-]*)\\s*$")
private val CLOSING_FENCE = Regex("^\\s*```\\s*$")
private val HEADING = Regex("^(#{1,6})\\s+(.*)$")
private val HRULE = Regex("^\\s*([-*_])(\\s*\\1){2,}\\s*$")
private val IMAGE = Regex("^\\s*!\\[([^\\]]*)]\\([^)]*\\)\\s*$")
private val TABLE_RULE = Regex("^\\s*\\|?\\s*:?-{3,}:?\\s*(\\|\\s*:?-{3,}:?\\s*)*\\|?\\s*$")
private val QUOTE = Regex("^\\s*>")
private val CHECK = Regex("^\\s*[-*+]\\s+\\[( |x|X)]\\s+.*$")
private val BULLET = Regex("^\\s*[-*+]\\s+.*$")
private val ORDERED = Regex("^\\s*\\d+[.)]\\s+.*$")

private fun fenced(lines: List<String>, start: Int, lang: String, out: MutableList<Block>): Int {
    var j = start + 1
    val body = mutableListOf<String>()
    while (j < lines.size && !CLOSING_FENCE.matches(lines[j])) { body += lines[j]; j++ }
    val closed = j < lines.size
    val text = body.joinToString("\n")
    out += when (lang.lowercase()) {
        "chart" -> Block.ChartBlock(if (closed) Charts.parse(text) else ChartParse.Bad("still arriving"), text, closed)
        "mermaid" -> Charts.parseMermaidPie(text).let { if (it is ChartParse.Ok) Block.ChartBlock(it, text, closed) else Block.Code(lang, text, closed) }
        else -> Block.Code(lang, text, closed)
    }
    return if (closed) j + 1 else j
}

private fun quote(lines: List<String>, start: Int, out: MutableList<Block>): Int {
    var j = start
    val body = mutableListOf<String>()
    while (j < lines.size && QUOTE.containsMatchIn(lines[j])) { body += lines[j].replaceFirst(Regex("^\\s*>\\s?"), ""); j++ }
    out += Block.Quote(body.joinToString("\n"))
    return j
}

private fun checklist(lines: List<String>, start: Int, out: MutableList<Block>): Int {
    var j = start
    val items = mutableListOf<Pair<Boolean, String>>()
    while (j < lines.size && CHECK.matches(lines[j])) {
        val m = Regex("^\\s*[-*+]\\s+\\[( |x|X)]\\s+(.*)$").matchEntire(lines[j])!!
        items += (m.groupValues[1] != " ") to m.groupValues[2].trim(); j++
    }
    out += Block.Checklist(items)
    return j
}

private fun list(lines: List<String>, start: Int, out: MutableList<Block>): Int {
    val ordered = ORDERED.matches(lines[start])
    var j = start
    val items = mutableListOf<String>()
    val marker = if (ordered) ORDERED else BULLET
    while (j < lines.size && marker.matches(lines[j]) && !CHECK.matches(lines[j])) {
        items += lines[j].replaceFirst(Regex("^\\s*([-*+]|\\d+[.)])\\s+"), "").trim(); j++
    }
    out += Block.Bullets(ordered, items)
    return j
}

private fun table(lines: List<String>, start: Int, out: MutableList<Block>): Int {
    val headers = cells(lines[start])
    val aligns = cells(lines[start + 1]).map { c ->
        when { c.startsWith(":") && c.endsWith(":") -> Align.Center; c.endsWith(":") -> Align.End; else -> Align.Start }
    }
    var j = start + 2
    val rows = mutableListOf<List<String>>()
    while (j < lines.size && lines[j].isNotBlank() && lines[j].contains('|')) {
        val row = cells(lines[j])
        rows += List(headers.size) { row.getOrElse(it) { "" } }; j++
    }
    out += Block.Table(headers, List(headers.size) { aligns.getOrElse(it) { Align.Start } }, rows)
    return j
}

private fun cells(line: String): List<String> {
    val t = line.trim().removePrefix("|").removeSuffix("|")
    return t.split(Regex("(?<!\\\\)\\|")).map { it.replace("\\|", "|").trim() }
}

private fun paragraph(lines: List<String>, start: Int, out: MutableList<Block>): Int {
    var j = start
    val body = mutableListOf<String>()
    while (j < lines.size && lines[j].isNotBlank() && !startsBlock(lines, j)) { body += lines[j]; j++ }
    if (body.isEmpty()) { body += lines[start]; j = start + 1 }
    out += Block.Paragraph(body.joinToString("\n"))
    return j
}

private fun startsBlock(lines: List<String>, i: Int): Boolean {
    val l = lines[i]
    return i > 0 && (FENCE.matches(l) || HEADING.matches(l) || QUOTE.containsMatchIn(l) || CHECK.matches(l) || BULLET.matches(l) || ORDERED.matches(l) ||
        IMAGE.matches(l) || (l.contains('|') && i + 1 < lines.size && TABLE_RULE.matches(lines[i + 1])))
}
