package example.hq.spike.ui.chat.blocks

import java.net.URLDecoder

/**
 * How a chat message points at a vault note, read exactly as apps/hq-web/src/lib/noteRefs.ts and the
 * path cleaning in vaultNoteStore.ts read it, so a reference opens the same note on the phone and in the browser.
 */
object NoteRefs {
    private const val MAX_REF = 300
    private val NOT_A_PATH = Regex("[\\r\\n\\t`<>*?\"|]")
    private val FORBIDDEN_START = listOf("http://", "https://", "//", "mailto:", "javascript:", "data:")

    /** The vault path for a backticked reference like `Notebooks/Projects/Plan.md`, or null when it is just code. */
    fun fromCode(text: String): String? {
        if (text != text.trim() || text.length > MAX_REF) return null
        if (NOT_A_PATH.containsMatchIn(text) || !text.contains('/') || !text.lowercase().endsWith(".md")) return null
        return sanitize(text)
    }

    /** The vault path a markdown link points at, or null for an ordinary link. */
    fun fromLink(href: String): String? {
        val h = href.trim()
        if (h.startsWith("/api/note")) return sanitize(h)
        val noFragment = h.substringBefore('#')
        val isScheme = Regex("^(vault|note):", RegexOption.IGNORE_CASE).containsMatchIn(noFragment)
        val isMarkdownFile = !h.contains("://") && noFragment.lowercase().endsWith(".md")
        if (!isScheme && !isMarkdownFile) return null
        return sanitize(safeDecode(noFragment))
    }

    /** `[[Note]]` or `[[folder/Note|Label]]` as (target, label). */
    fun wikilink(inner: String): Pair<String, String> {
        val parts = inner.split('|')
        val target = parts[0].trim()
        return target to (parts.getOrNull(1) ?: parts[0]).trim()
    }

    fun sanitize(raw: String?): String? {
        var p = raw?.trim().orEmpty()
        if (p.isEmpty()) return null
        if (p.startsWith("[[") && p.endsWith("]]")) p = p.substring(2, p.length - 2).trim()
        if (p.contains('|')) p = p.substringBefore('|').trim()
        val lower = p.lowercase()
        if (lower.startsWith("vault:")) p = p.substring(6).trim() else if (lower.startsWith("note:")) p = p.substring(5).trim()
        if (p.startsWith("/api/note")) {
            p = p.substringAfter("?path=", "").substringBefore('&').let { safeDecode(it) }
        }
        p = p.trim()
        if (p.isEmpty()) return null
        val l = p.lowercase()
        if (FORBIDDEN_START.any { l.startsWith(it) }) return null
        if (p.startsWith("/") || p.startsWith("\\") || p.contains("..")) return null
        return p
    }

    /** The name shown for a note path: the file name without `.md`. */
    fun title(path: String): String = path.substringAfterLast('/').removeSuffix(".md")

    /** "Projects › Agent-HQ" for `Notebooks/Projects/Agent-HQ/Plan.md`: the folders under the top one. */
    fun folderTrail(path: String): String {
        val folders = path.substringBeforeLast('/', "").split('/').filter { it.isNotEmpty() }
        return folders.drop(1).joinToString(" › ").ifEmpty { folders.joinToString(" › ") }
    }

    private fun safeDecode(s: String) = try { URLDecoder.decode(s, "UTF-8") } catch (e: IllegalArgumentException) { s }
}

/** Notes start with a YAML block between two `---` lines; the reader shows only what follows it. */
object NoteText {
    private val FENCE = Regex("^---\\s*$")

    fun withoutFrontmatter(text: String): String {
        val lines = text.lines()
        if (lines.isEmpty() || !FENCE.matches(lines[0])) return text
        val end = (1 until lines.size).firstOrNull { FENCE.matches(lines[it]) } ?: return text
        return lines.drop(end + 1).joinToString("\n").trimStart('\n')
    }
}
