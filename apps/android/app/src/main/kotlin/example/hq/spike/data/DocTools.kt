package example.hq.spike.data

/** One block that [DocTools.plan] would rewrite: its place and hash as listed, and the markdown it would become. */
data class Replacement(val row: BlockRow, val markdown: String)

/** What a find and replace did: how many blocks it changed, and how many hq refused (a block that changed meanwhile). */
data class ReplaceResult(val changed: Int, val refused: Int)

/** The pure part of the block editor's table and find-and-replace tools. */
object DocTools {
    /** Blocks whose words a find and replace may change. Code, sheets, plots, tiles and comments are left alone. */
    private fun isProse(kind: String) = kind.startsWith("heading") || kind in PROSE

    private val PROSE = setOf("paragraph", "list", "numbered list", "quote", "table")

    const val MAX_COLUMNS = 8
    const val MAX_ROWS = 20

    /** A markdown table with [headers] on top and [rows] empty rows under them. */
    fun tableMarkdown(headers: List<String>, rows: Int): String {
        val cols = headers.size.coerceAtLeast(1)
        val head = headers.joinToString(" | ", prefix = "| ", postfix = " |")
        val rule = List(cols) { "---" }.joinToString(" | ", prefix = "| ", postfix = " |")
        val blank = List(cols) { " " }.joinToString(" | ", prefix = "| ", postfix = " |")
        return (listOf(head, rule) + List(rows.coerceAtLeast(1)) { blank }).joinToString("\n")
    }

    /** How many times [find] appears in [text] (not overlapping). */
    fun count(text: String, find: String): Int {
        if (find.isEmpty()) return 0
        var n = 0
        var at = text.indexOf(find)
        while (at >= 0) { n++; at = text.indexOf(find, at + find.length) }
        return n
    }

    /** The blocks that hold [find], each with its text rewritten to use [with]. Nothing when [find] is empty. */
    fun plan(blocks: List<BlockRow>, find: String, with: String): List<Replacement> {
        if (find.isEmpty()) return emptyList()
        return blocks.filter { isProse(it.kind) && it.src.contains(find) }.map { Replacement(it, it.src.replace(find, with)) }
    }

    /** The number of occurrences of [find] across the blocks a find and replace may change. */
    fun occurrences(blocks: List<BlockRow>, find: String): Int = blocks.filter { isProse(it.kind) }.sumOf { count(it.src, find) }
}

/** Adds a table of [columns] named [headers] and [rows] empty rows after [after] (or at the top when null). */
suspend fun HqClient.suiteInsertTable(path: String, after: BlockRow?, headers: List<String>, rows: Int): List<BlockRow> =
    suiteInsertBlock(path, after?.index, after?.hash, DocTools.tableMarkdown(headers, rows))

/**
 * Replaces [find] with [with] in every prose block of the note, one block edit each, so a block that changed since it was read is
 * refused by hq on its own and the rest still go through.
 */
suspend fun HqClient.suiteFindReplace(path: String, find: String, with: String): ReplaceResult {
    var changed = 0
    var refused = 0
    for (r in DocTools.plan(suiteBlocks(path), find, with)) {
        try {
            suiteEditBlock(path, r.row.index, r.row.hash, r.markdown)
            changed++
        } catch (e: HqUnavailable) {
            if (e.status == 0) throw e
            refused++
        }
    }
    return ReplaceResult(changed, refused)
}
