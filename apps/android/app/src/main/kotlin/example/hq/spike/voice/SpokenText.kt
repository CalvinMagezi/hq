package example.hq.spike.voice

/**
 * Turns a reply that streams in as markdown into plain sentences a voice can read, one at a time, so speech
 * starts after the first sentence instead of after the whole reply.
 */
class SpokenText(private val longestWait: Int = LONGEST_WAIT) {
    private val pending = StringBuilder()
    private var inCode = false

    /** Adds streamed text and returns every sentence that is now complete, already cleaned for speech. */
    fun add(delta: String): List<String> {
        pending.append(delta)
        val out = mutableListOf<String>()
        while (true) {
            val cut = nextCut() ?: break
            emit(pending.substring(0, cut), out)
            pending.delete(0, cut)
        }
        return out
    }

    /** The reply ended: whatever is left is the last sentence. */
    fun finish(): List<String> {
        val out = mutableListOf<String>()
        emit(pending.toString(), out)
        pending.clear()
        return out
    }

    /** Where the first complete piece ends: a line break, a sentence end followed by a space, or a long run with no end. */
    private fun nextCut(): Int? {
        val newline = pending.indexOf('\n')
        if (newline >= 0) return newline + 1
        for (i in 0 until pending.length - 1) {
            if (pending[i] in SENTENCE_ENDS && pending[i + 1].isWhitespace()) return i + 2
        }
        if (pending.length < longestWait || inCode) return null
        val space = pending.lastIndexOf(" ", longestWait)
        return if (space > 0) space + 1 else null
    }

    private fun emit(raw: String, out: MutableList<String>) {
        for (line in raw.split('\n')) {
            if (line.trimStart().startsWith(FENCE)) {
                inCode = !inCode
                if (inCode) out += CODE_SAID
                continue
            }
            if (inCode) continue
            val spoken = speakable(line)
            if (spoken.isNotEmpty()) out += spoken
        }
    }

    companion object {
        /** A sentence that never ends is spoken from here on, at the last space, so a list or a run-on reply is not held back. */
        const val LONGEST_WAIT = 160
        private const val FENCE = "```"
        private const val CODE_SAID = "There is code in the reply; it is on the screen."
        private val SENTENCE_ENDS = setOf('.', '!', '?', ';', ':')
        private val LINK = Regex("""\[([^\]]+)]\([^)]*\)""")
        private val URL = Regex("""https?://\S+""")
        private val INLINE_CODE = Regex("`([^`]*)`")
        private val EMPHASIS = Regex("""[*_~]{1,3}""")
        private val HEADING_OR_QUOTE = Regex("""^\s*(#{1,6}|>)\s*""")
        private val BULLET = Regex("""^\s*([-*+]|\d+[.)])\s+""")
        private val TABLE_RULE = Regex("""^\s*\|?[\s:|-]+\|?\s*$""")
        private val SPACES = Regex("""\s+""")

        /** One line of markdown as words: links read as their text, addresses and formatting marks dropped, table cells joined. */
        fun speakable(line: String): String {
            if (TABLE_RULE.matches(line) && line.contains('-')) return ""
            var s = HEADING_OR_QUOTE.replace(line, "")
            s = BULLET.replace(s, "")
            s = LINK.replace(s) { it.groupValues[1] }
            s = URL.replace(s, "a link")
            s = INLINE_CODE.replace(s) { it.groupValues[1] }
            s = EMPHASIS.replace(s, "")
            s = s.replace('|', ',').trim().trim(',').trim()
            return SPACES.replace(s, " ")
        }
    }
}
