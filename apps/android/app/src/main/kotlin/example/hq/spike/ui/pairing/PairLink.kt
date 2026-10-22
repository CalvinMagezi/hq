package example.hq.spike.ui.pairing

/**
 * Reads a pairing link from the outside world: the stock camera, a browser, another app. A link
 * only ever names a screen to show. It is never acted on, because anyone can send one, and following
 * it would pair this phone with whoever made it (`docs/security/PAIRING.md`, section 10).
 */
object PairLink {
    private const val SCHEME = "hqpair"
    private const val ROUTE = "ticket"
    /** The same bound `hq` applies when it parses a link (`MAX_LINK_CHARS` in crates/hq-bridge/src/ticket.rs). */
    const val MAX_CHARS = 2000
    private const val MIN_BODY_CHARS = 16

    /** The link text if [text] looks like one of ours, else null. The ticket inside is checked by `hq`. */
    fun accept(text: String?): String? {
        val t = text?.trim() ?: return null
        if (t.length > MAX_CHARS) return null
        val prefix = "$SCHEME://$ROUTE/"
        if (!t.startsWith(prefix, ignoreCase = true)) return null
        val body = t.substring(prefix.length)
        return t.takeIf { body.length >= MIN_BODY_CHARS && body.all(::urlSafe) }
    }

    private fun urlSafe(c: Char) = c.isLetterOrDigit() && c.code < 128 || c == '-' || c == '_'

    /** Short enough to read aloud: the first eight characters of a device id. */
    fun shortId(id: String): String = id.take(SHORT_ID_CHARS)

    private const val SHORT_ID_CHARS = 8
}
