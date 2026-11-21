package example.hq.spike.data

/**
 * What a card's editor shows and keeps. [text] is the card's own words, as markdown. [kept] are the blocks the editor does not
 * show (the `<!-- notes: ... -->` and look comments, and review comments) and sends back unchanged, because hq's replace of a
 * card drops everything the new text does not carry.
 */
data class CardBody(val text: String, val kept: List<String>) {
    /** The markdown to send when the person's text is [edited]: that text, then the blocks that were kept. */
    fun markdown(edited: String): String = (listOf(edited.trim()) + kept).filter { it.isNotBlank() }.joinToString("\n\n")
}

/** Finds a card's blocks among a note's blocks. Cards start at a `## ` heading, as hq finds them. */
object SlideBodies {
    private fun isHead(b: BlockRow) = b.kind == "heading 2" && b.src.startsWith("## ")

    private fun isKept(b: BlockRow) = b.kind == "comment" || (b.kind == "html" && b.src.trimStart().startsWith("<!--"))

    /** The body of the `card`th card (zero-based), or null when the note has no such card. */
    fun bodyOf(blocks: List<BlockRow>, card: Int): CardBody? {
        val heads = blocks.indices.filter { isHead(blocks[it]) }
        val start = heads.getOrNull(card) ?: return null
        val end = heads.getOrNull(card + 1) ?: blocks.size
        val (kept, shown) = blocks.subList(start + 1, end).partition { isKept(it) }
        return CardBody(shown.joinToString("\n\n") { it.src.trim() }, kept.map { it.src.trim() })
    }

    /** The first [lines] lines of a card's text, for the small card on the list. */
    fun preview(body: CardBody?, lines: Int): String =
        body?.text?.lineSequence()?.map { it.trim() }?.filter { it.isNotEmpty() }?.take(lines)?.joinToString("\n").orEmpty()
}

/** The choices the card editor offers, as hq words them on the wire. */
object CardStyles {
    val layouts = listOf("content", "two-column")

    /** `null` is "no transition set". */
    val transitions: List<String?> = listOf(null, "none", "fade", "push", "wipe", "split", "cover", "dissolve")

    /** `fade slow` is the transition `fade`: the speed is not offered here, and is dropped when the transition is saved. */
    fun transitionKind(wire: String?): String? = wire?.trim()?.substringBefore(' ')?.takeIf { it.isNotEmpty() }
}

/** The kind of HQ Suite note a front matter says it is (`doc`, `sheet`, `slides`, `dashboard`), or null for a plain note. */
fun suiteKindOf(noteText: String): String? {
    val lines = noteText.lines()
    if (lines.firstOrNull()?.trim() != "---") return null
    val end = (1 until lines.size).firstOrNull { lines[it].trim() == "---" } ?: return null
    val line = lines.subList(1, end).firstOrNull { it.startsWith("suite_kind:") } ?: return null
    return line.substringAfter(':').trim().trim('"', '\'').lowercase().takeIf { it.isNotEmpty() }
}
