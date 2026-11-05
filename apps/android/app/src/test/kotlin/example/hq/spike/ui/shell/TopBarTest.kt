package example.hq.spike.ui.shell

import androidx.compose.ui.test.assertIsDisplayed
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import example.hq.spike.testing.Screenshots
import example.hq.spike.ui.theme.HqTheme
import example.hq.spike.ui.theme.Mts
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** The first Compose UI and screenshot tests: they prove the D-11 setup runs on the JVM. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w390dp-h120dp-xxhdpi")
class TopBarTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun shows_the_place_and_its_status_and_opens_search() {
        var searched = false
        rule.setContent {
            HqTheme { TopBar("Chat", "Connected", Mts.InkMuted, onSearch = { searched = true }, onSettings = {}) }
        }
        rule.onNodeWithText("Chat").assertIsDisplayed()
        rule.onNodeWithText("Connected").assertIsDisplayed()
        rule.onNodeWithContentDescription("Search").performClick()
        assertTrue(searched)
    }

    @Test fun looks_as_recorded() {
        rule.setContent {
            HqTheme { TopBar("Chat", "Connected", Mts.InkMuted, onSearch = {}, onSettings = {}) }
        }
        Screenshots.assertMatches(rule, "top_bar_connected")
    }
}
