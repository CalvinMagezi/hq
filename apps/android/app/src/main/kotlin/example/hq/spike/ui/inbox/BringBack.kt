package example.hq.spike.ui.inbox

/** The chat a Later offer's inbox item points to: its `metadata.link` is `/chat?thread=<id>`. */
object ChatLink {
    private val THREAD = Regex("""^/chat\?(?:[^#]*&)?thread=([^&#]+)""")

    /** The thread id, or null for no link, another page, or a link with no usable id. */
    fun threadOf(link: String?): String? {
        val raw = THREAD.find(link.orEmpty().trim())?.groupValues?.get(1) ?: return null
        val id = runCatching { java.net.URLDecoder.decode(raw, "UTF-8") }.getOrNull()?.trim()
        return id?.takeIf { it.isNotEmpty() }
    }
}

/** One "Bring it back" at a time per row: a second tap while the first is on its way sends nothing. */
class BringBackGuard {
    private val busy = mutableSetOf<String>()

    /** True when the caller may go ahead; it must call [end] when done. */
    fun begin(id: String): Boolean = busy.add(id)

    fun end(id: String) {
        busy.remove(id)
    }
}
