package example.hq.spike.ui.search

import example.hq.spike.ui.notes.SearchHit
import org.junit.Assert.assertEquals
import org.junit.Test

class SearchWordsTest {
    private val hit = SearchHit("Notebooks/Inbox/A.md", "A", "Inbox", "")

    @Test fun chipsShowACountOnlyOnceSomethingIsTyped() {
        assertEquals("Notes", SearchWords.chip(SearchScope.Notes, 3, false))
        assertEquals("Notes 3", SearchWords.chip(SearchScope.Notes, 3, true))
    }

    @Test fun allCountsEveryKind() {
        val ui = SearchUi(query = "a", notes = listOf(hit, hit), tasks = emptyList(), chats = emptyList())
        assertEquals(2, SearchWords.countOf(ui, SearchScope.All))
        assertEquals(0, SearchWords.countOf(ui, SearchScope.Tasks))
    }

    @Test fun noMatchWaitsForNotesAndIsNotSaidWhenNotesCouldNotBeAsked() {
        assertEquals(false, SearchWords.noMatch(SearchUi(query = "")))
        assertEquals(false, SearchWords.noMatch(SearchUi(query = "a", loading = true)))
        assertEquals(false, SearchWords.noMatch(SearchUi(query = "a", notesFailed = true)))
        assertEquals(true, SearchWords.noMatch(SearchUi(query = "a")))
        assertEquals(false, SearchWords.noMatch(SearchUi(query = "a", notes = listOf(hit))))
    }

    @Test fun aTasksOnlySearchIsEmptyEvenWhenNotesFound() {
        assertEquals(true, SearchWords.noMatch(SearchUi(query = "a", scope = SearchScope.Tasks, notes = listOf(hit))))
    }
}
