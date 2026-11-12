package example.hq.spike.ui.shell

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import example.hq.spike.ui.theme.HqTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w390dp-h400dp-xxhdpi")
class BottomBarTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun theBarBadgeStopsAtNine() {
        assertEquals("9", Counts.navBadge(9))
        assertEquals("9+", Counts.navBadge(10))
        assertEquals("9+", Counts.navBadge(160))
    }

    @Test fun inboxShowsNinePlusFromTen() {
        rule.setContent { HqTheme { BottomBar(Place.Chat, inboxBadge = 12) {} } }
        rule.onNodeWithText("9+").assertIsDisplayed()
    }

    @Test fun notesDotShowsWithoutANumber() {
        rule.setContent { HqTheme { BottomBar(Place.Chat, inboxBadge = 0, notesDot = true) {} } }
        rule.onNodeWithTag("notes-dot", useUnmergedTree = true).assertIsDisplayed()
    }

    @Test fun noDotUnlessAsked() {
        rule.setContent { HqTheme { BottomBar(Place.Chat, inboxBadge = 0) {} } }
        assertEquals(0, rule.onAllNodesWithTagCount("notes-dot"))
    }

    @Test fun theNotesDotHasWordsForTalkBack() {
        rule.setContent { HqTheme { BottomBar(Place.Chat, inboxBadge = 0, notesDot = true) {} } }
        rule.onNodeWithContentDescription("New in your Vault", useUnmergedTree = true).assertIsDisplayed()
    }

    /** Owner decision: Search lives on Vault (Notes) and Chat only; the other places have their own find or filter. */
    @Test fun searchIsHiddenWhenAPlaceHasNone() {
        rule.setContent { HqTheme { TopBar("Inbox", "Connected", androidx.compose.ui.graphics.Color.Gray, onSearch = {}, onSettings = {}, showSearch = false) } }
        rule.onNodeWithText("Inbox").assertIsDisplayed()
        assertEquals(0, rule.onAllNodesDescribed("Search"))
    }
}

private fun androidx.compose.ui.test.junit4.ComposeTestRule.onAllNodesWithTagCount(tag: String) =
    onAllNodes(androidx.compose.ui.test.hasTestTag(tag), useUnmergedTree = true).fetchSemanticsNodes().size

private fun androidx.compose.ui.test.junit4.ComposeTestRule.onAllNodesDescribed(desc: String) =
    onAllNodes(androidx.compose.ui.test.hasContentDescription(desc)).fetchSemanticsNodes().size
