package example.hq.spike.ui.webhost

import androidx.compose.runtime.staticCompositionLocalOf
import example.hq.spike.HubService
import java.net.URI
import java.net.URISyntaxException

/** The web app's pages the app shows inside its own screen: the only ones the host will load. */
enum class WebPage(val slug: String, val title: String, val path: String) {
    Home("home", "Home", "/vault"),
    Usage("usage", "Usage", "/usage"),
    Board("board", "Board", "/tasks?view=board"),
    Timeline("timeline", "Timeline", "/tasks?view=timeline");

    companion object {
        fun fromSlug(slug: String?): WebPage? = entries.firstOrNull { it.slug == slug?.lowercase() }
    }
}

/** Where a screen asks for one of those pages to open; set once by the shell. */
val LocalOpenWeb = staticCompositionLocalOf<(WebPage) -> Unit> { {} }

/** What the host decides without a WebView in hand, so it can be tested. */
object WebHost {
    val ORIGIN = "http://127.0.0.1:${HubService.PORT}"
    const val TOKEN_KEY = "hq-web-token"
    /** The web app's own page title, which says nothing about the page, so the bar keeps its own word. */
    const val WEB_TITLE = "Agent HQ Vault"
    private const val EMBED = "host=android"

    /** The address to load: the page, with the flag that hides the web app's own header, bars and overlays. */
    fun url(page: WebPage): String = ORIGIN + page.path + (if ('?' in page.path) "&" else "?") + EMBED

    /** Only the local hq loads in the host; anything else opens in the browser. */
    fun allows(url: String?): Boolean {
        val u = try { URI(url ?: return false) } catch (e: URISyntaxException) { return false }
        return u.scheme == "http" && u.host == "127.0.0.1" && u.port == HubService.PORT
    }

    /** Only a web link goes to the browser; a link for another app (an intent: address, a file) is dropped. */
    fun opensInBrowser(url: String?): Boolean {
        val scheme = try { URI(url ?: return false).scheme?.lowercase() } catch (e: URISyntaxException) { return false }
        return scheme == "https" || scheme == "http" || scheme == "mailto"
    }

    fun barTitle(page: WebPage, webTitle: String?): String {
        val t = webTitle?.trim().orEmpty()
        // A page with no title of its own is named by its address, which is not a title.
        return if (t.isEmpty() || t == WEB_TITLE || '/' in t || t == "about:blank") page.title else t
    }

    /** The script that gives the page its sign-in before it runs: the token goes into the page's own storage, never into the address. */
    fun tokenScript(token: String): String = "localStorage.setItem('$TOKEN_KEY', ${jsString(token)})"

    /** A JavaScript string literal that cannot be closed early whatever it holds. */
    fun jsString(s: String): String = buildString {
        append('"')
        for (ch in s) when {
            ch == '"' || ch == '\\' -> append('\\').append(ch)
            ch == '<' || ch == '>' || ch == ' ' || ch == ' ' || ch.code < 0x20 -> append("\\u%04x".format(ch.code))
            else -> append(ch)
        }
        append('"')
    }

    fun failedTitle(page: WebPage) = "Couldn't load ${page.title}"
    const val FAILED_BODY = "HQ did not answer. Check that it is running, then try again."
    const val RENDER_BODY = "The page stopped working. Try again."
    const val TRY_AGAIN = "Try again"
}
