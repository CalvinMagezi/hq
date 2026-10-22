package example.hq.spike.ui.shell

import java.net.URI
import java.net.URISyntaxException

/** Where an hq:// link goes: a place, and what to open in it. A link only navigates; nothing it carries is ever acted on. */
data class LinkTarget(
    val place: Place = Place.Chat,
    /** A chat, inbox item, task or helper id. */
    val id: String? = null,
    /** A note's path under the vault. */
    val path: String? = null,
    /** A Settings screen name, from [Links.SETTINGS_SCREENS]. */
    val screen: String? = null,
    val search: Boolean = false,
)

/** The links of the design's "Navigation and Back" table: hq://chat/{id}, inbox/{id}, tasks/{id}, notes/{path}, helpers[/{id}], settings/{screen}, search. hq://web/{page} joins them with the WebView host. */
object Links {
    const val SCHEME = "hq"
    const val EXTRA = "link"
    private const val MAX_PATH = 512
    private val ID = Regex("[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}")

    val SETTINGS_SCREENS = setOf("home", "safety", "keys", "messaging", "claude", "devices", "spending", "about")

    /** Never throws. Null for anything that is not one of our links, so a stray or hostile link does nothing at all. */
    fun parse(raw: String?): LinkTarget? {
        if (raw.isNullOrBlank()) return null
        val u = try { URI(raw.trim()) } catch (e: URISyntaxException) { return null }
        if (u.scheme?.lowercase() != SCHEME) return null
        val host = u.host?.lowercase() ?: u.authority?.lowercase() ?: return null
        val rest = (u.path ?: "").removePrefix("/").removeSuffix("/")
        return when (host) {
            "chat" -> LinkTarget(Place.Chat, id = id(rest))
            "inbox" -> LinkTarget(Place.Inbox, id = id(rest))
            "tasks" -> LinkTarget(Place.Tasks, id = id(rest))
            "helpers" -> LinkTarget(Place.Helpers, id = id(rest))
            "notes" -> LinkTarget(Place.Notes, path = notePath(rest))
            "settings" -> rest.lowercase().ifEmpty { "home" }.takeIf { it in SETTINGS_SCREENS }?.let { LinkTarget(Place.Chat, screen = it) }
            "search" -> LinkTarget(Place.Notes, search = true)
            else -> null
        }
    }

    private fun id(s: String): String? = s.takeIf { ID.matches(it) }

    /** A vault path: relative, no way out of the vault and no control characters. The server decides whether it exists and may be read. */
    private fun notePath(s: String): String? {
        if (s.isBlank() || s.length > MAX_PATH || s.startsWith("/") || "\\" in s || s.any { it.isISOControl() }) return null
        if (s.split('/').any { it == ".." || it == "." || it.isEmpty() }) return null
        return s
    }
}
