package example.hq.spike.ui.search

import example.hq.spike.data.Connection
import example.hq.spike.testing.testWords
import example.hq.spike.ui.helpers.HelperCard
import example.hq.spike.ui.helpers.HelperState
import example.hq.spike.ui.notes.SearchHit
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class SearchWordsTest {
    private val w = testWords()
    private fun hit(p: String) = SearchHit("Notebooks/$p.md", p.substringAfterLast('/'), "", "")
    private fun helper(title: String, where: String = "Studio Mac") = HelperCard("h", title, "Claude", where, HelperState.Working, "", "", null, "s")

    @Test fun chipsNameTheScopeBeforeAnythingIsTyped() {
        SearchScope.entries.forEach { assertEquals(SearchWords.scope(w, it), SearchWords.chip(w, SearchUi(), it)) }
        assertEquals(listOf("All", "Vault", "Tasks", "Chats", "Helpers"), SearchScope.entries.map { SearchWords.scope(w, it) })
    }

    @Test fun typedChipsCountEveryKindAndShowZero() {
        val ui = SearchUi(query = "a", notes = listOf(hit("A/x"), hit("B/y")), helpers = listOf(helper("t")))
        assertEquals("All 3", SearchWords.chip(w, ui, SearchScope.All))
        assertEquals("Vault 2", SearchWords.chip(w, ui, SearchScope.Notes))
        assertEquals("Tasks 0", SearchWords.chip(w, ui, SearchScope.Tasks))
        assertEquals("Chats 0", SearchWords.chip(w, ui, SearchScope.Chats))
        assertEquals("Helpers 1", SearchWords.chip(w, ui, SearchScope.Helpers))
    }

    @Test fun notesAndChatsShowNoNumberWhenHqCouldNotBeAsked() {
        val ui = SearchUi(query = "a", hqDown = true, notesFail = NotesFail.Stopped)
        assertEquals("Vault", SearchWords.chip(w, ui, SearchScope.Notes))
        assertEquals("Chats", SearchWords.chip(w, ui, SearchScope.Chats))
        assertEquals("Tasks 0", SearchWords.chip(w, ui, SearchScope.Tasks))
    }

    @Test fun aFullPageOfNotesIsShownAsAtLeast() {
        val ui = SearchUi(query = "a", notes = (1..SearchViewModel.NOTE_HITS).map { hit("F/n$it") })
        assertEquals("Vault 30+", SearchWords.chip(w, ui, SearchScope.Notes))
    }

    @Test fun aFailedAskOfNotesIsNeverNothingMatchesThat() {
        assertEquals(false, SearchWords.noMatch(SearchUi(query = "")))
        assertEquals(false, SearchWords.noMatch(SearchUi(query = "a", loading = true)))
        assertEquals(false, SearchWords.noMatch(SearchUi(query = "a", notesFail = NotesFail.Timeout)))
        assertEquals(false, SearchWords.noMatch(SearchUi(query = "a", scope = SearchScope.Notes, notesFail = NotesFail.Timeout)))
        assertEquals(true, SearchWords.noMatch(SearchUi(query = "a")))
        assertEquals(true, SearchWords.noMatch(SearchUi(query = "a", scope = SearchScope.Tasks, notes = listOf(hit("A/x")), notesFail = NotesFail.Timeout)))
        assertEquals(true, SearchWords.noMatch(SearchUi(query = "a", scope = SearchScope.Helpers)))
    }

    @Test fun eachScopeHasItsOwnNoMatchLine() {
        assertEquals("No notes match that.", SearchWords.none(w, SearchScope.Notes))
        assertEquals("No helpers match that.", SearchWords.none(w, SearchScope.Helpers))
        assertEquals("Nothing matches that.", SearchWords.none(w, SearchScope.All))
    }

    @Test fun theFailureLineFollowsTheStateHqWasIn() {
        assertEquals(NotesFail.Starting, NotesFail.of(Connection.Starting(0, 5, null)))
        assertEquals(NotesFail.Restarting, NotesFail.of(Connection.Restarting(3)))
        assertEquals(NotesFail.Stopped, NotesFail.of(Connection.Stopped(0, null)))
        assertEquals(NotesFail.Timeout, NotesFail.of(Connection.Connected))
        assertEquals(NotesFail.Timeout, NotesFail.of(Connection.Offline(0)))
    }

    @Test fun theAskPillTakesAtMostTenNotes() {
        assertNull(SearchWords.ask(w, 0))
        assertEquals("Ask HQ about this note", SearchWords.ask(w, 1))
        assertEquals("Ask HQ about these 7 notes", SearchWords.ask(w, 7))
        assertEquals("Ask HQ about these 10 notes", SearchWords.ask(w, 10))
        assertEquals("Ask HQ about the first 10 notes", SearchWords.ask(w, 13))
    }

    @Test fun folderChipsCountOnlyWhenHqReturnedLessThanTheLimit() {
        val counted = SearchUi(query = "a", scope = SearchScope.Notes, folders = mapOf("Projects" to 4, "Inbox" to 4, "Ideas" to 1))
        assertEquals(listOf<Pair<String?, Int>>(null to 9, "Inbox" to 4, "Projects" to 4, "Ideas" to 1), SearchWords.folderChips(counted))
        assertTrue(SearchWords.folderChips(counted.copy(folders = null)).isEmpty())
        assertTrue(SearchWords.folderChips(counted.copy(scope = SearchScope.All)).isEmpty())
    }

    @Test fun aFolderStartIsReadAsATrail() {
        assertEquals("Projects / Agent-HQ", SearchWords.trail("Projects/Agent-HQ"))
    }

    @Test fun theSavedTimeIsNamedOnlyWhenThereIsOne() {
        assertNull(SearchWords.savedLine(w, SearchUi()))
        assertTrue(SearchWords.savedLine(w, SearchUi(savedAtMs = System.currentTimeMillis()))!!.startsWith("Tasks below are from the copy saved on this phone at "))
    }

    @Test fun aServerHelperSaysWhereItIs() {
        val line = SearchWords.helperLine(w, SearchUi(server = true, savedAtMs = System.currentTimeMillis()), helper("Fix it"))
        assertTrue(line, line.contains("on your server HQ") && line.contains("saved at "))
        assertEquals("Fix it \u00b7 Studio Mac \u00b7 Working on it", SearchWords.helperLine(w, SearchUi(), helper("Fix it")))
    }

    @Test fun matchedWordsAreBoldWithNoBackground() {
        val s = bolded("The Tunnel hub", "tunnel hub")
        assertEquals(2, s.spanStyles.size)
        assertTrue(s.spanStyles.all { it.item.fontWeight != null && it.item.background == androidx.compose.ui.graphics.Color.Unspecified })
    }

    @Test fun clearingRecentLeavesNothingAndTheFolderIsTheTopOne() {
        assertTrue(SearchMatch.clearRecent().isEmpty())
        assertEquals("Projects", SearchMatch.folderOf("Notebooks/Projects/Agent-HQ/x.md"))
    }

    @Test fun helpersMatchOnTitleAndHelperName() {
        val all = listOf(helper("Fix the tunnel"), helper("Write docs"))
        assertEquals(listOf("Fix the tunnel"), SearchMatch.helpers(all, "tunnel").map { it.title })
        assertEquals(2, SearchMatch.helpers(all, "claude").size)
    }
}
