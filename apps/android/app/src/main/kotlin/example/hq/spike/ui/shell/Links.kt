package example.hq.spike.ui.shell

import example.hq.spike.ui.webhost.WebPage
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
    /** A web page the app shows in its own screen, by [example.hq.spike.ui.webhost.WebPage.slug]. */
    val web: String? = null,
)

/** The links of the design's "Navigation and Back" table: hq://chat/{id}, inbox/{id}, tasks/{id}, notes/{path}, helpers[/{id}], settings/{screen}, search. hq://web/{home|usage|board|timeline} opens a web page in the app's own screen. */
object Links {
    const val SCHEME = "hq"
    const val EXTRA = "link"
    private const val MAX_PATH = 512
    private val ID = Regex("[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}")

    val SETTINGS_SCREENS = setOf("home", "safety", "keys", "messaging", "claude", "devices", "spending", "about", "background", "keeprunning", "notifications", "origins", "connectedtools", "computers", "capturewidgets", "storedkeys")

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
            "web" -> webPage(rest, u.query)?.let { LinkTarget(Place.Chat, web = it.slug) }
            else -> null
        }
    }

    /** The link to a vault note. Each part of the path is percent-encoded, since a title with a space or a # would not parse as a link otherwise. */
    fun noteLink(path: String): String = "$SCHEME://notes/" + path.split('/').joinToString("/") { java.net.URLEncoder.encode(it, "UTF-8").replace("+", "%20") }

    /** Only the four pages the host shows: hq://web/home, usage, board, timeline (and /vault and /tasks?view= as the web app spells them). */
    private fun webPage(rest: String, query: String?): WebPage? = when (rest.lowercase()) {
        "vault" -> WebPage.Home
        "tasks" -> when (query) { "view=board" -> WebPage.Board; "view=timeline" -> WebPage.Timeline; else -> null }
        else -> WebPage.fromSlug(rest)
    }

    private fun id(s: String): String? = s.takeIf { ID.matches(it) }

    /** A vault path: relative, no way out of the vault and no control characters. The server decides whether it exists and may be read. */
    private fun notePath(s: String): String? {
        if (s.isBlank() || s.length > MAX_PATH || s.startsWith("/") || "\\" in s || s.any { it.isISOControl() }) return null
        if (s.split('/').any { it == ".." || it == "." || it.isEmpty() }) return null
        return s
    }
}
