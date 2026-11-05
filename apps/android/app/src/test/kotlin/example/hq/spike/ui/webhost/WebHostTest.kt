package example.hq.spike.ui.webhost

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class WebHostTest {
    @Test fun everyPageIsLoadedWithTheEmbedFlagAndOnlyThePagesTheHostKnows() {
        assertEquals("http://127.0.0.1:5678/settings/spending?embed=1", WebHost.url(WebPage.Usage))
        assertEquals("http://127.0.0.1:5678/tasks?view=board&embed=1", WebHost.url(WebPage.Board))
        assertEquals("http://127.0.0.1:5678/tasks?view=timeline&embed=1", WebHost.url(WebPage.Timeline))
        assertNull(WebPage.fromSlug("home"))
        assertEquals(WebPage.Timeline, WebPage.fromSlug("TimeLine"))
        assertNull(WebPage.fromSlug("settings"))
        assertNull(WebPage.fromSlug(null))
    }

    @Test fun onlyTheLocalHqLoadsInTheHost() {
        assertTrue(WebHost.allows("http://127.0.0.1:5678/usage?embed=1"))
        assertFalse(WebHost.allows("https://127.0.0.1:5678/usage"))
        assertFalse(WebHost.allows("http://127.0.0.1:5679/usage"))
        assertFalse(WebHost.allows("http://127.0.0.1.example.com:5678/"))
        assertFalse(WebHost.allows("http://localhost:5678/"))
        assertFalse(WebHost.allows("http://127.0.0.1:5678@example.com/"))
        assertFalse(WebHost.allows("javascript:alert(1)"))
        assertFalse(WebHost.allows("not a url"))
        assertFalse(WebHost.allows(null))
    }

    @Test fun theRetiredHomeDoesNotLoadAndOpensNotes() {
        assertFalse(WebHost.allows("http://127.0.0.1:5678/vault"))
        assertFalse(WebHost.allows("http://127.0.0.1:5678/vault/Projects"))
        assertTrue(WebHost.allows("http://127.0.0.1:5678/tasks?view=board"))
        assertTrue(WebHost.allows("http://127.0.0.1:5678/vaulted"))
        assertTrue(WebHost.opensNotes("http://127.0.0.1:5678/vault"))
        assertTrue(WebHost.opensNotes("http://127.0.0.1:5678/vault/x"))
        assertFalse(WebHost.opensNotes("http://127.0.0.1:5678/tasks"))
        assertFalse(WebHost.opensNotes("http://127.0.0.1:5679/vault"))
    }

    @Test fun onlyWebAndMailLinksGoToTheBrowser() {
        assertTrue(WebHost.opensInBrowser("https://example.com/a"))
        assertTrue(WebHost.opensInBrowser("mailto:a@example.com"))
        assertFalse(WebHost.opensInBrowser("intent://scan/#Intent;scheme=zxing;end"))
        assertFalse(WebHost.opensInBrowser("file:///data/data/x"))
        assertFalse(WebHost.opensInBrowser("javascript:alert(1)"))
    }

    @Test fun theTokenScriptCannotBeBrokenOutOfWhateverTheTokenHolds() {
        assertEquals("localStorage.setItem('hq-web-token', \"abc-123_\")", WebHost.tokenScript("abc-123_"))
        val nasty = WebHost.jsString("a\"b\\c</script>\n ")
        assertFalse(nasty.contains('\n'))
        assertFalse(nasty.contains("</script>"))
        assertTrue(nasty.startsWith("\"") && nasty.endsWith("\""))
        assertEquals("\"a\\\"b\\\\c\\u003c/script\\u003e\\u000a\\u2028\"", nasty)
    }
}
