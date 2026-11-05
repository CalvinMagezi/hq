package example.hq.spike.ui.settings

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import example.hq.spike.ui.theme.HqTheme
import example.hq.spike.update.ChannelManifest
import example.hq.spike.update.UpdateState
import example.hq.spike.update.Updater
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w390dp-h1200dp-xxhdpi")
class UpdateAndDataPagesTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
    private val manifest = ChannelManifest("1.4.0", 140, "https://example.com/hq.apk", "0".repeat(64), 26, notes = mapOf("en" to listOf("Faster chat")), size = 40_000_000L)
    private val facts = Updater.Facts(installedVersion = "1.3.0", installedBuild = 130, release = "15")

    @Test fun theUpdatePageNamesBothVersionsAndTheNotes() {
        var opened = 0
        rule.setContent { HqTheme { UpdatePage(SettingsUi(update = UpdateState.Available(manifest), updateFacts = facts), SettingsActions(onUpdatePageOpened = { opened++ })) } }
        rule.onNodeWithText("On this phone").assertIsDisplayed()
        rule.onNodeWithText("1.3.0 · Build 130").assertIsDisplayed()
        rule.onNodeWithText("New version").assertIsDisplayed()
        rule.onNodeWithText("1.4.0 · Build 140").assertIsDisplayed()
        rule.onNodeWithText("Faster chat").assertIsDisplayed()
        assertEquals(1, opened)
    }

    @Test fun withNothingToInstallThePageSaysWhereItStands() {
        rule.setContent { HqTheme { UpdatePage(SettingsUi(update = UpdateState.UpToDate, updateFacts = facts), SettingsActions()) } }
        rule.onNodeWithText("Up to date").assertIsDisplayed()
        rule.onAllNodesWithText("What is new").assertCountEquals(0)
    }

    @Test fun aPhoneThatIsTooOldSaysSoAndKeepsWorking() {
        rule.setContent { HqTheme { UpdatePage(SettingsUi(update = UpdateState.NeedsNewerAndroid(34, "1.4.0"), updateFacts = facts), SettingsActions()) } }
        rule.onNodeWithText("This update needs a newer Android").assertIsDisplayed()
    }

    @Test fun theUpdateCardOpensThePage() {
        val pages = mutableListOf<SettingsPage>()
        rule.setContent { HqTheme { UpdateCard(SettingsUi(update = UpdateState.Available(manifest)), SettingsActions(onPage = { pages += it })) } }
        rule.onNodeWithText("Update ready").assertIsDisplayed()
        rule.onNodeWithText("See update").performClick()
        assertEquals(listOf(SettingsPage.Update), pages)
    }

    @Test fun mobileDataAsksAboutAnyNetworkAndCanForgetWifi() {
        val asked = mutableListOf<Boolean>()
        var forgot = 0
        rule.setContent { HqTheme { Page { MobileData(SettingsUi(wifiFree = 2, wifiPaid = 1), SettingsActions(onAskAnyNetwork = { asked += it }, onForgetWifi = { forgot++ })) } } }
                rule.onNodeWithText("2 free · 1 paid by the MB").assertIsDisplayed()
        rule.onNodeWithText("Ask before big downloads on any network").performClick()
        assertEquals(listOf(true), asked)
        rule.onNodeWithText("Forget").performClick()
        assertEquals(1, forgot)
    }

    @Test fun mobileDataHasNoForgetWhenNoWifiIsKnown() {
        rule.setContent { HqTheme { Page { MobileData(SettingsUi(), SettingsActions()) } } }
        rule.onAllNodesWithText("Forget").assertCountEquals(0)
    }

    @Test fun theAssistantRowOnHomeShowsItsStatusAndOpensThePage() {
        val pages = mutableListOf<SettingsPage>()
        val row = HomeRow(SettingsPage.Assistant, "assistant", "Phone assistant", "Not set up")
        rule.setContent { HqTheme { SettingsHost(SettingsUi(open = true, loaded = true, phone = listOf(row)), SettingsActions(onPage = { pages += it })) } }
        rule.onNodeWithText("Phone assistant").performScrollTo().assertIsDisplayed()
        rule.onNodeWithText("Not set up").assertIsDisplayed()
        rule.onNodeWithText("Phone assistant").performClick()
        assertEquals(listOf(SettingsPage.Assistant), pages)
    }
}
