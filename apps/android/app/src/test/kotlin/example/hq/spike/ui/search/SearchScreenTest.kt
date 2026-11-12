package example.hq.spike.ui.search

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import example.hq.spike.ui.notes.SearchHit
import example.hq.spike.ui.theme.HqTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h900dp-xxhdpi")
class SearchScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
    private val hit = SearchHit("Notebooks/Projects/A.md", "Roadmap", "Projects", "the roadmap")

    private fun show(ui: SearchUi, actions: SearchActions = SearchActions(), goes: SearchGoes = SearchGoes({}, {}, {}, {}, {}, {}), stopped: Boolean = false) {
        rule.setContent { HqTheme { SearchScreen(ui, actions, goes, stopped) } }
    }

    @Test fun theScopeChipsShowBeforeAnythingIsTypedAsTabs() {
        var scope: SearchScope? = null
        show(SearchUi(open = true, scope = SearchScope.Notes), SearchActions(onScope = { scope = it }))
        listOf("All", "Vault", "Tasks", "Chats", "Helpers").forEach { rule.onNodeWithText(it).assertIsDisplayed() }
        rule.onNodeWithText("Vault").assertIsSelected()
        rule.onNodeWithText("Tasks").assertIsNotSelected()
        rule.onNodeWithText("Tasks").performClick()
        assertEquals(SearchScope.Tasks, scope)
    }

    @Test fun theFieldNamesWhereItLooks() {
        show(SearchUi(open = true, scope = SearchScope.Notes))
        rule.onNodeWithText("Search the Vault").assertExists()
    }

    @Test fun askHqShowsOnNotesWhenNotesAreFoundAndAttachesThem() {
        var asked: List<SearchHit>? = null
        show(SearchUi(open = true, query = "road", scope = SearchScope.Notes, notes = listOf(hit)), goes = SearchGoes({}, {}, {}, {}, { asked = it }, {}))
        rule.onNodeWithText("Ask HQ about this note").performClick()
        assertEquals(listOf(hit), asked)
    }

    @Test fun askHqIsNotOnTasks() {
        show(SearchUi(open = true, query = "road", scope = SearchScope.Tasks, notes = listOf(hit)))
        rule.onNodeWithText("Ask HQ about this note").assertDoesNotExist()
    }

    @Test fun aStoppedHqGivesTheFailureLineAndAStartButtonNotNothingMatches() {
        var started = 0
        show(SearchUi(open = true, query = "road", hqDown = true, notesFail = NotesFail.Stopped), goes = SearchGoes({}, {}, {}, {}, {}, { started++ }), stopped = true)
        rule.onNodeWithText("Couldn't search your Vault and chats. HQ has stopped. It will try again when HQ is back.").assertIsDisplayed()
        rule.onNodeWithText("Nothing matches that.").assertDoesNotExist()
        rule.onNodeWithText("Start HQ").performClick()
        assertEquals(1, started)
    }

    @Test fun aScopeWithNoMatchOffersSearchEverything() {
        var scope: SearchScope? = null
        show(SearchUi(open = true, query = "zzz", scope = SearchScope.Helpers), SearchActions(onScope = { scope = it }))
        rule.onNodeWithText("No helpers match that.").assertIsDisplayed()
        rule.onNodeWithText("Search everything").performClick()
        assertEquals(SearchScope.All, scope)
    }

    @Test fun aFolderStartShowsItsChipAndCaptionAndClearingSearchesAllNotes() {
        var folder: String? = "x"
        show(SearchUi(open = true, scope = SearchScope.Notes, folder = "Projects/Agent-HQ"), SearchActions(onFolder = { folder = it }))
        rule.onNodeWithText("In Projects / Agent-HQ").performClick()
        assertEquals(null, folder)
        rule.onNodeWithText("Searching only in Projects / Agent-HQ and the folders inside it.").assertIsDisplayed()
    }

    @Test fun clearingRecentSearchesActsAtOnceAndOffersUndo() {
        var undone = 0
        show(SearchUi(open = true, cleared = true), SearchActions(onUndoClear = { undone++ }))
        rule.onNodeWithText("Recent searches cleared").assertIsDisplayed()
        rule.onNodeWithText("Undo").performClick()
        assertEquals(1, undone)
    }

    @Test fun recentSearchesHaveAClearButton() {
        var cleared = 0
        show(SearchUi(open = true, recent = listOf("one", "two")), SearchActions(onClearRecent = { cleared++ }))
        rule.onNodeWithText("Clear recent searches").performClick()
        assertEquals(1, cleared)
    }
}
