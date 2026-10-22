package example.hq.spike.ui.shell

import example.hq.spike.ui.settings.SettingsPage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Every row of the design's "Navigation and Back" table, and what a link may not do. */
class LinksTest {
    private fun parse(raw: String?) = Links.parse(raw)!!

    @Test fun everyPlaceHasItsLink() {
        assertEquals(LinkTarget(Place.Chat, id = "t-1"), parse("hq://chat/t-1"))
        assertEquals(LinkTarget(Place.Inbox, id = "val_9f3a"), parse("hq://inbox/val_9f3a"))
        assertEquals(LinkTarget(Place.Tasks, id = "tk-1792633187805-071613"), parse("hq://tasks/tk-1792633187805-071613"))
        assertEquals(LinkTarget(Place.Helpers), parse("hq://helpers"))
        assertEquals(LinkTarget(Place.Helpers, id = "s-42"), parse("hq://helpers/s-42"))
        assertEquals(LinkTarget(Place.Chat), parse("hq://chat"))
    }

    @Test fun aTrailingSlashChangesNothing() {
        assertEquals("t-1", parse("hq://chat/t-1/").id)
        assertEquals("keys", parse("hq://settings/keys/").screen)
    }

    @Test fun theSchemeAndHostAreNotCaseSensitive() {
        assertEquals(LinkTarget(Place.Chat, id = "t-1"), parse("HQ://CHAT/t-1"))
    }

    @Test fun aNoteLinkCarriesItsPathDecoded() {
        assertEquals("Notebooks/Projects/Agent-HQ/Mobile-First-Roadmap-2026-10-20.md", parse("hq://notes/Notebooks/Projects/Agent-HQ/Mobile-First-Roadmap-2026-10-20.md").path)
        assertEquals("Notebooks/Projects/Samsung battery steps.md", parse("hq://notes/Notebooks/Projects/Samsung%20battery%20steps.md").path)
    }

    @Test fun aPathThatLeavesTheVaultIsDropped() {
        for (bad in listOf("hq://notes/../etc/passwd", "hq://notes/Notebooks/../../x", "hq://notes//abs", "hq://notes/a/./b", "hq://notes/a%00b", "hq://notes/a%5Cb")) {
            assertNull(bad, parse(bad).path)
            assertEquals(bad, Place.Notes, parse(bad).place)
        }
        assertNull(parse("hq://notes/" + "a".repeat(600)).path)
        assertNull(parse("hq://notes").path)
    }

    @Test fun settingsScreensAreAnAllowedSet() {
        assertEquals("keys", parse("hq://settings/keys").screen)
        assertEquals("spending", parse("hq://settings/Spending").screen)
        assertEquals("home", parse("hq://settings").screen)
        assertEquals(Place.Chat, parse("hq://settings/keys").place)
        assertNull(Links.parse("hq://settings/everything"))
    }

    @Test fun everyAllowedSettingsScreenIsAPageAndEveryPageIsAllowed() {
        // The set-up and pairing pages need a bot in hand, so a link cannot open them.
        assertEquals(SettingsPage.entries.filter { it != SettingsPage.SetupApp && it != SettingsPage.PairApp }.map { it.name.lowercase() }.toSet(), Links.SETTINGS_SCREENS)
    }

    @Test fun searchOpensTheSearch() {
        assertTrue(parse("hq://search").search)
    }

    @Test fun anIdThatIsNotAnIdIsDropped() {
        for (bad in listOf("hq://inbox/a b", "hq://tasks/" + "x".repeat(200), "hq://chat/%3Cscript%3E", "hq://chat/..", "hq://chat/.", "hq://chat/.hidden", "hq://chat/-x")) {
            assertNull(bad, Links.parse(bad)?.id)
        }
    }

    @Test fun whatIsNotOursOrNotALinkDoesNothing() {
        for (other in listOf(null, "", "  ", "https://example.com", "hq:/chat", "hq://", "hq://unknown/x", "hq://x", "not a url", "hq://[bad", "javascript:alert(1)", "hq:chat", "hq:///x", "hq://web", "hq://web/settings", "hq://web/tasks", "hq://web/tasks?view=list", "hq://web/../usage")) {
            assertNull(other.toString(), Links.parse(other))
        }
    }

    @Test fun oddButSyntacticLinksNeverThrow() {
        for (odd in listOf("hq://x@chat/1", "hq://chat?x=1", "hq://chat#frag", "hq://" + "a".repeat(100_000), "hq://chat/" + "%".repeat(50), "hq://c\u0000hat", "hq://chat/é")) {
            Links.parse(odd)
        }
        assertNotNull(Links.parse("hq://chat?x=1"))
    }

    @Test fun aWebLinkNamesOnlyOneOfTheFourPagesTheHostShows() {
        assertEquals("usage", Links.parse("hq://web/usage")?.web)
        assertEquals("home", Links.parse("hq://web/vault")?.web)
        assertEquals("home", Links.parse("hq://web/Home")?.web)
        assertEquals("board", Links.parse("hq://web/tasks?view=board")?.web)
        assertEquals("timeline", Links.parse("hq://web/timeline")?.web)
        assertEquals("timeline", Links.parse("hq://web/tasks?view=timeline")?.web)
    }
}
