package example.hq.spike.ui.notes

import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.hasSetTextAction
import example.hq.spike.data.HqClient
import example.hq.spike.data.suiteComments
import example.hq.spike.data.HqServices
import example.hq.spike.ui.theme.HqTheme
import example.hq.spike.ui.theme.Mts
import java.io.File
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The slide screen, the card editor and the comments dialog run for real against a live hq: the screens draw what
 * the server sent and what the person does in them reaches the note. A diagnostic that skips unless `HQ_LIVE_URL`
 * and `HQ_LIVE_VAULT` are set (see SuiteLiveTest); it swaps the app singleton's client for one on that address.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-xxhdpi")
class SuiteEditorsLiveRenderTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
    private val url = System.getenv("HQ_LIVE_URL")
    private val vault = System.getenv("HQ_LIVE_VAULT")

    /**
     * The app singleton without its constructor, which starts polling loops that need a real phone, holding only a
     * client on the live address. The screens under test read nothing else from it.
     */
    private fun useLiveClient() {
        val unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe").apply { isAccessible = true }
        val unsafe = unsafeField.get(null)
        val services = unsafe.javaClass.getMethod("allocateInstance", Class::class.java).invoke(unsafe, HqServices::class.java)
        HqServices::class.java.getDeclaredField("client").apply { isAccessible = true }.set(services, HqClient({ "" }, url))
        HqServices::class.java.getDeclaredField("instance").apply { isAccessible = true }.set(null, services)
    }

    private fun waitFor(text: String) = rule.waitUntil(timeoutMillis = 60_000) { rule.onAllNodesWithText(text, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
    private fun note(name: String) = File(vault, "Live/$name.md").readText()

    @Test fun theSlideScreenListsTheDeckAndTheCardEditorChangesTheNote() {
        assumeTrue(url != null && vault != null)
        useLiveClient()
        rule.setContent { HqTheme { Column(Modifier.fillMaxSize().background(Mts.Surface000)) { SlidesView("Live/deck.md", null, {}, {}) } } }
        waitFor("One")
        rule.onNodeWithText("Two", useUnmergedTree = true).assertIsDisplayed()
        rule.onNodeWithText("Three", useUnmergedTree = true).assertIsDisplayed()
        // Open the first card: the editor offers the card's own title, which the person changes and saves.
        rule.onNodeWithText("One", useUnmergedTree = true).performClick()
        rule.waitUntil(timeoutMillis = 60_000) { rule.onAllNodes(hasSetTextAction(), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
        rule.onAllNodes(hasSetTextAction(), useUnmergedTree = true)[0].performTextReplacement("Opening")
        rule.onNodeWithText("Save", useUnmergedTree = true).performClick()
        rule.waitUntil(timeoutMillis = 60_000) { note("deck").contains("## Opening") }
        assertTrue(note("deck"), !note("deck").contains("## One\n"))
    }

    @Test fun theCommentsDialogShowsTheCommentsAndResolveReachesTheNote() {
        assumeTrue(url != null && vault != null)
        useLiveClient()
        val client = HqServices.get(androidx.test.core.app.ApplicationProvider.getApplicationContext<android.content.Context>()).client
        // One call first, so the first request's set-up time (connection, class loading) is not spent inside the screen's wait.
        assertTrue(runBlocking { client.suiteComments("Live/deck.md") }.any { it.text == "Is this final?" })
        rule.setContent { HqTheme { CommentsDialog("Live/deck.md") {} } }
        rule.waitForIdle()
        waitFor("Is this final?")
        rule.onNodeWithText("Resolve", useUnmergedTree = true).performClick()
        rule.waitUntil(timeoutMillis = 60_000) { note("deck").contains("state=resolved") }
        assertTrue(note("deck"), note("deck").contains("Is this final?"))
    }
}
