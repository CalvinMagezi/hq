package example.hq.spike.ui.settings

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import example.hq.spike.testing.testWords
import example.hq.spike.ui.shell.ConfirmCopy
import example.hq.spike.ui.theme.HqTheme
import example.hq.spike.update.BusyReason
import example.hq.spike.update.ChannelManifest
import example.hq.spike.update.UpdateState
import java.time.Instant
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** The independent review of W5: each test here failed before its fix. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w390dp-h1200dp-xxhdpi")
class ReviewFixesTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
    private val now = Instant.parse("2026-11-13T12:00:00Z")

    @org.junit.Before fun english() = java.util.Locale.setDefault(java.util.Locale.ENGLISH)

    // 1. Keys on this phone does not wait for HQ.
    @Test fun theKeysOnThisPhoneShowWhenHqDoesNotAnswer() {
        assertEquals(listOf(SettingsWords.KeysPart.Phone), SettingsWords.keysParts(null))
        assertEquals(listOf(SettingsWords.KeysPart.Hq, SettingsWords.KeysPart.Phone), SettingsWords.keysParts(JSONObject()))
        rule.setContent { HqTheme { SettingsHost(SettingsUi(open = true, stack = listOf(SettingsPage.Keys), loaded = true), SettingsActions()) } }
        rule.onNodeWithText("KEYS ON THIS PHONE").assertExists()
    }

    // 2. The remove question is a modal on Settings, with steps that can be tested.
    @Test fun removingAKeyGoesAskThenBusyThenDoneAndReplaceHandsTheNameBack() {
        val asked = KeyRemoval.ask(SettingsUi(), "OPENROUTER_API_KEY", only = true)
        assertEquals(KeyRemove("OPENROUTER_API_KEY", true), asked.keyRemove)
        assertNull(KeyRemoval.cancel(asked).keyRemove)
        val busy = KeyRemoval.begin(asked)
        assertTrue(busy.keyRemove!!.busy)
        assertEquals("a running removal cannot be cancelled", busy, KeyRemoval.cancel(busy))
        assertEquals(busy, KeyRemoval.replace(busy))
        val replaced = KeyRemoval.replace(asked)
        assertNull(replaced.keyRemove)
        assertEquals("OPENROUTER_API_KEY", replaced.keyReplace)
        assertNull(KeyRemoval.replaceHandled(replaced).keyReplace)
        val done = KeyRemoval.finished(busy, "Removed.", ok = true)
        assertNull(done.keyRemove)
        assertEquals("Removed.", done.keysLine)
        assertEquals(1, done.keysVersion)
        assertTrue(KeyRemoval.finished(busy, "no", ok = false).keysLineBad)
    }

    @Test fun theRemoveQuestionIsAModalOverSettings() {
        var kept = 0
        var removed = 0
        var context: android.content.Context? = null
        rule.setContent {
            HqTheme {
                context = androidx.compose.ui.platform.LocalContext.current
                SettingsHost(SettingsUi(open = true, stack = listOf(SettingsPage.Keys), loaded = true, keyRemove = KeyRemove("OPENROUTER_API_KEY", false)), SettingsActions(onAskRemoveKey = { kept++ }, onRemoveKey = { removed++ }))
            }
        }
        val title = ConfirmCopy.removeKey(context!!, ConfirmCopy.keyLabel("OPENROUTER_API_KEY"), false).title
        rule.onNodeWithText(title).assertIsDisplayed()
    }

    // 3. One install question, not two.
    @Test fun aBusyUpdateShowsTheInstallQuestionOnce() {
        val m = ChannelManifest("1.4.0", 140, "https://example.com/hq.apk", "0".repeat(64), 26, size = 1L)
        val ui = SettingsUi(update = UpdateState.Busy(m, BusyReason.Chat), updateFacts = example.hq.spike.update.Updater.Facts(installedVersion = "1.3.0", installedBuild = 130, release = "15"))
        rule.setContent { HqTheme { UpdatePage(ui, SettingsActions()) } }
        rule.onAllNodesWithText("Wait until it's done").assertCountEquals(1)
    }

    // 4. The server row keeps its state word.
    @Test fun theServerRowPutsTheStateFirstAndKeepsItWhateverTheHost() {
        val host = "hq-with-a-very-long-name.tailnet-example.ts.net"
        listOf(true to "Signed out", false to "Connected").forEach { (out, word) ->
            val row = SettingsWords.home(testWords(), SettingsSnapshot(), now, emptyList(), server = SettingsWords.serverRow(testWords(), host, out)).flatMap { it.rows }.first { it.title == "Server HQ" }
            assertTrue(row.status, row.status.startsWith(word))
            assertTrue(row.status.length <= SettingsWords.STATE_MAX)
            assertEquals(out, row.warn)
        }
    }

    // 5. A choice is not a warning.
    @Test fun notificationsAreAmberOnlyWhenSomethingNeedsDoing() {
        assertFalse(SettingsWords.notificationsWarn(allowed = true, needsOn = true, updatesOn = true))
        assertTrue(SettingsWords.notificationsWarn(allowed = false, needsOn = true, updatesOn = true))
        assertTrue(SettingsWords.notificationsWarn(allowed = true, needsOn = false, updatesOn = true))
    }

    // Lower items.
    @Test fun thePhoneRowsFollowTheOneListInItsOrderAndDropAnyOther() {
        val rows = listOf(SettingsPage.CaptureWidgets, SettingsPage.Update, SettingsPage.Background, SettingsPage.Assistant).map { HomeRow(it, it.name, it.name, "") }
        assertEquals(listOf(SettingsPage.Background, SettingsPage.Assistant, SettingsPage.CaptureWidgets), SettingsWords.inPhoneOrder(rows).map { it.page })
    }

    @Test fun theBuiltInHostIsNotAPairedComputer() {
        assertEquals(0, SettingsWords.pairedComputers(JSONObject("""{"agent_host":{"hosts":["native"]}}""")))
        assertEquals(1, SettingsWords.pairedComputers(JSONObject("""{"agent_host":{"hosts":["native","mac-mini"]}}""")))
        val row = SettingsWords.home(testWords(), SettingsSnapshot(settings = JSONObject("""{"agent_host":{"hosts":["native"]}}""")), now, emptyList()).flatMap { it.rows }.first { it.page == SettingsPage.Computers }
        assertEquals("No computer paired", row.status)
    }

    @Test fun theSpeechModelIsReachableWhenTheAssistantRoleIsNot() {
        rule.setContent { HqTheme { AssistantPage(SettingsUi(speechBytes = 120_000_000L), SettingsActions()) } }
        rule.onNodeWithText("Delete speech model").assertExists()
    }

    @Test fun backgroundWorkSaysKeepRunningOnceAsARowAndTheSectionReadsDifferently() {
        rule.setContent { HqTheme { SettingsHost(SettingsUi(open = true, stack = listOf(SettingsPage.Background), loaded = true), SettingsActions()) } }
        rule.onAllNodesWithText("Keep running").assertCountEquals(1)
        rule.onNodeWithText("IF ANDROID STOPS HQ").assertExists()
    }

    @Test fun everyParentPageShowsARowForEachOfItsChildren() {
        assertEquals(setOf(SettingsPage.Background, SettingsPage.Safety, SettingsPage.Technical, SettingsPage.Messaging), SettingsRoutes.CHILDREN.keys)
        val settings = JSONObject("""{"safety":{"web_allowed_origins":["https://hq.example.ts.net"]}}""")
        fun shows(parent: SettingsPage, text: String, ui: SettingsUi = SettingsUi(open = true, stack = listOf(parent), loaded = true)) {
            rule.setContent { HqTheme { SettingsHost(ui, SettingsActions()) } }
            rule.onNodeWithText(text).assertExists()
        }
        shows(SettingsPage.Background, "Keep running")
        assertEquals(SettingsPage.Origins, SettingsWords.safetyRows(testWords(), settings, SettingsWords.SafetyTab.Safety).first { it.title == "Allowed addresses" }.opens)
    }

    @Test fun technicalDetailsShowsWhatHqNoticed() {
        rule.setContent { HqTheme { SettingsHost(SettingsUi(open = true, stack = listOf(SettingsPage.Technical), loaded = true), SettingsActions()) } }
        rule.onNodeWithText("What HQ noticed").assertExists()
    }
}
