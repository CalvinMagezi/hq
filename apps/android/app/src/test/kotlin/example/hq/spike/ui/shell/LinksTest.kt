package example.hq.spike.ui.shell

import example.hq.spike.ui.search.SearchScope
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
        assertEquals(LinkTarget(Place.Helpers, github = true), parse("hq://helpers/github"))
        assertEquals(LinkTarget(Place.Chat), parse("hq://chat"))
        assertEquals(LinkTarget(Place.Notes), parse("hq://vault"))
        assertEquals(LinkTarget(Place.Notes), parse("hq://notes"))
    }

    @Test fun theAssistantPageAndTheComposerHaveLinks() {
        assertEquals("assistant", parse("hq://settings/assistant").screen)
        assertEquals("speech", parse("hq://settings/speech").screen)
        assertTrue(parse("hq://chat?compose=1").compose)
        assertEquals(false, parse("hq://chat").compose)
    }

    @Test fun theUpdatePageHasALink() = assertEquals("update", parse("hq://settings/update").screen)

    @Test fun aTrailingSlashChangesNothing() {
        assertEquals("t-1", parse("hq://chat/t-1/").id)
        assertEquals("keys", parse("hq://settings/keys/").screen)
    }

    @Test fun theSchemeAndHostAreNotCaseSensitive() {
        assertEquals(LinkTarget(Place.Chat, id = "t-1"), parse("HQ://CHAT/t-1"))
    }

    @Test fun aNoteLinkCarriesItsPathDecoded() {
        assertEquals("Notebooks/Projects/Agent-HQ/Mobile-First-Roadmap-2026-10-20.md", parse("hq://vault/Notebooks/Projects/Agent-HQ/Mobile-First-Roadmap-2026-10-20.md").path)
        assertEquals("Notebooks/Projects/Agent-HQ/Mobile-First-Roadmap-2026-10-20.md", parse("hq://notes/Notebooks/Projects/Agent-HQ/Mobile-First-Roadmap-2026-10-20.md").path)
        assertEquals("Notebooks/Projects/Samsung battery steps.md", parse("hq://vault/Notebooks/Projects/Samsung%20battery%20steps.md").path)
        assertEquals("Notebooks/Projects/Samsung battery steps.md", parse("hq://notes/Notebooks/Projects/Samsung%20battery%20steps.md").path)
    }

    @Test fun aPathThatLeavesTheVaultIsDropped() {
        for (bad in listOf("hq://vault/../etc/passwd", "hq://notes/../etc/passwd", "hq://vault/Notebooks/../../x", "hq://notes/Notebooks/../../x", "hq://vault//abs", "hq://notes//abs", "hq://vault/a/./b", "hq://notes/a/./b", "hq://vault/a%00b", "hq://notes/a%00b", "hq://vault/a%5Cb", "hq://notes/a%5Cb")) {
            assertNull(bad, parse(bad).path)
            assertEquals(bad, Place.Notes, parse(bad).place)
        }
        assertNull(parse("hq://vault/" + "a".repeat(600)).path)
        assertNull(parse("hq://notes/" + "a".repeat(600)).path)
        assertNull(parse("hq://vault").path)
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
        // The set-up and pairing pages need a bot in hand, so a link cannot open them. Technical details and What HQ noticed open only from About.
        assertEquals(SettingsPage.entries.filter { it != SettingsPage.SetupApp && it != SettingsPage.PairApp && it != SettingsPage.Technical && it != SettingsPage.Noticed }.map { it.name.lowercase() }.toSet() + example.hq.spike.ui.settings.SettingsRoutes.ALIASES.keys, Links.SETTINGS_SCREENS)
    }

    @Test fun searchOpensTheSearchOnItsScope() {
        assertEquals(SearchScope.All, parse("hq://search").search)
        assertEquals(SearchScope.Notes, parse("hq://search/notes").search)
        assertEquals(SearchScope.Tasks, parse("hq://search/Tasks").search)
        assertEquals(SearchScope.All, parse("hq://search/nonsense").search)
        assertEquals(SearchScope.All, parse("hq://search?q=x").search)
    }

    @Test fun theAwayPageAndANoteByIdHaveLinks() {
        assertEquals(LinkTarget(Place.Tasks, away = true), parse("hq://tasks/away"))
        assertEquals(LinkTarget(Place.Notes, noteId = "abc123"), parse("hq://vault/n/abc123"))
        assertEquals(LinkTarget(Place.Notes, noteId = "abc123"), parse("hq://notes/n/abc123"))
        assertNull(Links.parse("hq://vault/n/a b")?.noteId)
        assertNull(Links.parse("hq://vault/n/..")?.noteId)
        assertNull(Links.parse("hq://notes/n/a b")?.noteId)
        assertNull(Links.parse("hq://notes/n/..")?.noteId)
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

    @Test fun theRetiredHomeOpensNotesWithNoWebPage() {
        for (link in listOf("hq://web/home", "hq://web/vault", "hq://web/Home")) {
            assertEquals(link, LinkTarget(Place.Notes), Links.parse(link))
        }
    }

    @Test fun aWebLinkNamesOnlyOneOfTheThreePagesTheHostShows() {
        assertEquals("usage", Links.parse("hq://web/usage")?.web)
        assertEquals("board", Links.parse("hq://web/tasks?view=board")?.web)
        assertEquals("timeline", Links.parse("hq://web/timeline")?.web)
        assertEquals("timeline", Links.parse("hq://web/tasks?view=timeline")?.web)
    }
}
