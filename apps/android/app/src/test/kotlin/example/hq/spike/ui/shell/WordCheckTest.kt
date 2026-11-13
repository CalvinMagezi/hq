package example.hq.spike.ui.shell

import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onRoot
import example.hq.spike.ui.helpers.HelpersActions
import example.hq.spike.ui.helpers.HelpersScreen
import example.hq.spike.ui.helpers.helpersFixture
import example.hq.spike.ui.settings.SettingsActions
import example.hq.spike.ui.settings.SettingsHost
import example.hq.spike.ui.settings.settingsFixture
import example.hq.spike.ui.tasks.TasksActions
import example.hq.spike.ui.tasks.TasksScreen
import example.hq.spike.ui.tasks.tasksFixture
import example.hq.spike.ui.theme.HqTheme
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Plain words, acceptance 4: the screens the owner reads, rendered from their fixtures, hold none of the words
 * the glossary retires. A node tagged "technical" is the one place a raw name may show, and is skipped.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w390dp-h2400dp-xxhdpi")
class WordCheckTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val banned = listOf(
        "lowercase hq" to Regex("(?<![\\w/.@-])hq(?![\\w/.-])"),
        "edge worker" to Regex("edge worker", RegexOption.IGNORE_CASE),
        "coding agent" to Regex("coding agent", RegexOption.IGNORE_CASE),
        "harness" to Regex("harness", RegexOption.IGNORE_CASE),
        "backend" to Regex("backend", RegexOption.IGNORE_CASE),
        "provider:" to Regex("provider:", RegexOption.IGNORE_CASE),
        "model:" to Regex("(?<![\\w])model:", RegexOption.IGNORE_CASE),
        "config" to Regex("\\bconfig\\b", RegexOption.IGNORE_CASE),
        "yaml" to Regex("yaml", RegexOption.IGNORE_CASE),
        "VPN" to Regex("\\bVPN\\b"),
        "n0" to Regex("\\bn0\\b"),
        "relay" to Regex("\\brelay\\b", RegexOption.IGNORE_CASE),
        "bridge" to Regex("\\bbridge\\b", RegexOption.IGNORE_CASE),
        "an ALL_CAPS name" to Regex("\\b[A-Z0-9]+_[A-Z0-9_]+\\b"),
        "a file path" to Regex("(?<![A-Za-z])(/[\\w.-]+){2,}"),
        "an HTTP status" to Regex("\\bHTTP \\d{3}\\b"),
        "Live as a status" to Regex("^Live$"),
        "Later on a button" to Regex("^Later$"),
        "a semicolon" to Regex(";"),
    )

    /** The key-name hint is the exact name HQ looks for, a thing to type rather than a word to read. */
    private fun owned(text: String) = text.isNotBlank() && text != example.hq.spike.ui.firstrun.FirstRunWords.KEYS_NAME_HINT

    /** Every sentence of [text] that runs past 15 words. */
    private fun longSentences(text: String): List<String> =
        text.split(Regex("(?<=[.!?])\\s+")).filter { it.trim().split(Regex("\\s+")).size > MAX_WORDS }

    private fun texts(): List<String> = rule.onRoot(useUnmergedTree = true).fetchSemanticsNode().let { root ->
        buildList {
            fun walk(node: androidx.compose.ui.semantics.SemanticsNode) {
                val config = node.config
                if (config.getOrNull(SemanticsProperties.TestTag) == "technical") return
                config.getOrNull(SemanticsProperties.Text)?.forEach { add(it.text) }
                config.getOrNull(SemanticsProperties.EditableText)?.let { add(it.text) }
                node.children.forEach(::walk)
            }
            walk(root)
        }
    }

    private fun findings(screen: String): List<String> = buildList {
        for (text in texts().filter(::owned)) {
            banned.forEach { (name, rx) -> if (rx.containsMatchIn(text)) add("$screen: $name in \"$text\"") }
            longSentences(text).forEach { add("$screen: sentence over $MAX_WORDS words: \"$it\"") }
        }
    }

    /** Shows each fixture in turn in the one composition and collects what the owner would read. */
    private fun <T : Any> check(names: List<String>, fixture: (String) -> T?, show: @androidx.compose.runtime.Composable (T) -> Unit) {
        var current by mutableStateOf<T?>(null)
        rule.setContent { HqTheme { current?.let { show(it) } } }
        val found = names.flatMap { name ->
            val ui = fixture(name) ?: return@flatMap emptyList()
            rule.runOnUiThread { current = ui }
            rule.waitForIdle()
            findings(name)
        }
        assertTrue(found.joinToString("\n"), found.isEmpty())
    }

    @Test fun theSettingsPagesUseThePlainWords() {
        val names = listOf("s-home", "s-safety", "s-time", "s-helpers", "s-keys", "s-chat", "s-spend", "s-spend-exceeded", "s-tools", "s-background", "s-notif", "s-keep-bad")
        check(names, ::settingsFixture) { SettingsHost(it, SettingsActions()) }
    }

    @Test fun theTasksScreensUseThePlainWords() {
        check(listOf("t-all", "t-today"), ::tasksFixture) { TasksScreen(it, TasksActions()) }
    }

    @Test fun theHelpersListUsesThePlainWords() {
        check(listOf("h-list", "h-needs"), ::helpersFixture) { HelpersScreen(it, HelpersActions()) }
    }

    private companion object {
        const val MAX_WORDS = 15
    }
}
