package example.hq.spike.ui.webhost

import androidx.annotation.StringRes
import androidx.compose.runtime.staticCompositionLocalOf
import example.hq.spike.R
import example.hq.spike.HubService
import java.net.URI
import java.net.URISyntaxException

/** The web app's pages the app shows inside its own screen: the only ones the host will load. */
enum class WebPage(val slug: String, @StringRes val title: Int, @StringRes val subtitle: Int, val path: String) {
    Usage("usage", R.string.place_usage, R.string.web_line_usage, "/settings/spending"),
    Board("board", R.string.web_board, R.string.web_line_board, "/tasks?view=board"),
    Timeline("timeline", R.string.web_timeline, R.string.web_line_timeline, "/tasks?view=timeline");

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
    private const val EMBED = "embed=1"

    /** The address to load: the page, with the flag that hides the web app's own header, bars and overlays. */
    fun url(page: WebPage): String = ORIGIN + page.path + (if ('?' in page.path) "&" else "?") + EMBED

    /** Only the local hq loads in the host; anything else opens in the browser. */
    fun allows(url: String?): Boolean {
        val u = try { URI(url ?: return false) } catch (e: URISyntaxException) { return false }
        return u.scheme == "http" && u.host == "127.0.0.1" && u.port == HubService.PORT && !isHome(u.path)
    }

    /** The retired web Home: a link to it inside a page does not load, and the shell opens Notes instead. */
    fun isHome(path: String?): Boolean = path == "/vault" || path?.startsWith("/vault/") == true

    /** True for a local address the host refuses only because it is the retired Home. */
    fun opensNotes(url: String?): Boolean {
        val u = try { URI(url ?: return false) } catch (e: URISyntaxException) { return false }
        return u.scheme == "http" && u.host == "127.0.0.1" && u.port == HubService.PORT && isHome(u.path)
    }

    /** Only a web link goes to the browser; a link for another app (an intent: address, a file) is dropped. */
    fun opensInBrowser(url: String?): Boolean {
        val scheme = try { URI(url ?: return false).scheme?.lowercase() } catch (e: URISyntaxException) { return false }
        return scheme == "https" || scheme == "http" || scheme == "mailto"
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
}
