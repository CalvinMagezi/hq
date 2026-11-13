package example.hq.spike.ui.shell

import example.hq.spike.ui.search.SearchScope
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
    /** hq://search[/{scope}]: the search screen, opened on this scope. */
    val search: SearchScope? = null,
    /** hq://tasks/away: the While you were away page. */
    val away: Boolean = false,
    /** hq://vault/n/{id} or hq://notes/n/{id}: a note by its id, the web's "Copy link" form. */
    val noteId: String? = null,
    /** A web page the app shows in its own screen, by [example.hq.spike.ui.webhost.WebPage.slug]. */
    val web: String? = null,
    /** hq://chat?compose=1: Chat with the composer focused and the keyboard up, never the microphone. */
    val compose: Boolean = false,
    /** hq://helpers/github: the HQ Coder sheet, where the browser comes back after connecting GitHub. */
    val github: Boolean = false,
)

/** The links of the design's "Navigation and Back" table: hq://chat/{id}, inbox/{id}, tasks/{id}, vault/{path}, vault/n/{id}, notes/{path}, notes/n/{id}, tasks/away, helpers[/{id}], helpers/github, settings/{screen}, search[/{scope}]. hq://web/{usage|board|timeline} opens a web page in the app's own screen; hq://web/home and vault open Vault. */
object Links {
    const val SCHEME = "hq"
    const val EXTRA = "link"
    private const val MAX_PATH = 512
    private val ID = Regex("[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}")

    val SETTINGS_SCREENS = example.hq.spike.ui.settings.SettingsRoutes.NAMES

    /** Never throws. Null for anything that is not one of our links, so a stray or hostile link does nothing at all. */
    fun parse(raw: String?): LinkTarget? {
        if (raw.isNullOrBlank()) return null
        val u = try { URI(raw.trim()) } catch (e: URISyntaxException) { return null }
        if (u.scheme?.lowercase() != SCHEME) return null
        val host = u.host?.lowercase() ?: u.authority?.lowercase() ?: return null
        val rest = (u.path ?: "").removePrefix("/").removeSuffix("/")
        return when (host) {
            "chat" -> LinkTarget(Place.Chat, id = id(rest), compose = u.query == "compose=1")
            "inbox" -> LinkTarget(Place.Inbox, id = id(rest))
            "tasks" -> if (rest == "away") LinkTarget(Place.Tasks, away = true) else LinkTarget(Place.Tasks, id = id(rest))
            "helpers" -> if (rest == "github") LinkTarget(Place.Helpers, github = true) else LinkTarget(Place.Helpers, id = id(rest))
            "notes", "vault" -> if (rest.startsWith("n/")) LinkTarget(Place.Notes, noteId = id(rest.removePrefix("n/"))) else LinkTarget(Place.Notes, path = notePath(rest))
            "settings" -> rest.lowercase().ifEmpty { "home" }.takeIf { it in SETTINGS_SCREENS }?.let { LinkTarget(Place.Chat, screen = it) }
            "search" -> LinkTarget(Place.Notes, search = SearchScope.entries.firstOrNull { it.name.equals(rest, true) } ?: SearchScope.All)
            "web" -> if (rest.lowercase() in RETIRED_HOME) LinkTarget(Place.Notes) else webPage(rest, u.query)?.let { LinkTarget(Place.Chat, web = it.slug) }
            else -> null
        }
    }

    /** The link to a vault note. Each part of the path is percent-encoded, since a title with a space or a # would not parse as a link otherwise. */
    fun noteLink(path: String): String = "$SCHEME://vault/" + path.split('/').joinToString("/") { java.net.URLEncoder.encode(it, "UTF-8").replace("+", "%20") }

    /** The web Home is retired: its links open Vault. */
    private val RETIRED_HOME = setOf("home", "vault")

    /** Only the three pages the host shows: hq://web/usage, board, timeline (and /tasks?view= as the web app spells them). */
    private fun webPage(rest: String, query: String?): WebPage? = when (rest.lowercase()) {
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
