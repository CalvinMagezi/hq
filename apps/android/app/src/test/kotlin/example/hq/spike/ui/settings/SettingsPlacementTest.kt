package example.hq.spike.ui.settings

import example.hq.spike.testing.testWords
import example.hq.spike.ui.shell.Links
import example.hq.spike.update.UpdateState
import java.time.Instant
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** Where every Settings page lives on the six-group Home, and that no old link lost its place. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class SettingsPlacementTest {
    private val now = Instant.parse("2026-11-13T12:00:00Z")

    @org.junit.Before fun english() = java.util.Locale.setDefault(java.util.Locale.ENGLISH)

    private fun phoneRows() = SettingsWords.PHONE_PAGES.map { HomeRow(it, it.name.lowercase(), it.name, "On") }

    private fun home(snap: SettingsSnapshot = SettingsSnapshot()) = SettingsWords.home(testWords(), snap, now, phoneRows())

    @Test fun homeHasTheSixNamedGroupsInOrder() {
        assertEquals(
            listOf("Work", "AI and money", "Chats and devices", "This phone", "Safety", "Your data and this app"),
            home().map { it.title },
        )
    }

    @Test fun eachGroupHoldsTheRowsTheBoardNames() {
        val titles = home().associate { it.title to it.rows.map { r -> r.title } }
        assertEquals(listOf("Tasks", "Helpers"), titles["Work"])
        assertEquals(listOf("AI service and keys", "Spending limits"), titles["AI and money"])
        assertEquals(listOf("Messaging apps", "Other devices", "Computers and tools", "GitHub"), titles["Chats and devices"])
        assertEquals(listOf("Safety and limits", "What HQ suggests"), titles["Safety"])
        assertEquals(listOf("Backup and moving phones", "Updates", "Technical details"), titles["Your data and this app"])
    }

    @Test fun everyPageIsReachableFromHomeDirectlyOrThroughAParent() {
        val onHome = home().flatMap { it.rows }.mapNotNull { it.page }.toSet()
        fun reachable(p: SettingsPage): Boolean = p == SettingsPage.Home || p in onHome || SettingsRoutes.PARENT[p]?.let { it != p && reachable(it) } == true
        val orphans = SettingsPage.entries.filterNot(::reachable)
        assertEquals("pages nobody can reach: $orphans", emptyList<SettingsPage>(), orphans)
    }

    @Test fun aPageOneLevelDownIsNeverAHomeRow() {
        val onHome = home().flatMap { it.rows }.mapNotNull { it.page }.toSet()
        assertTrue(SettingsRoutes.PARENT.keys.none { it in onHome })
    }

    @Test fun everyParentIsItselfOnHome() {
        val onHome = home().flatMap { it.rows }.mapNotNull { it.page }.toSet()
        assertTrue(SettingsRoutes.PARENT.values.all { it in onHome })
    }

    @Test fun keysSpeechAndConnectedToolsAreSectionsNotPages() {
        val names = SettingsPage.entries.map { it.name }
        assertTrue("StoredKeys" !in names && "Speech" !in names && "ConnectedTools" !in names)
        assertEquals(SettingsPage.Keys, SettingsRoutes.page("storedkeys"))
        assertEquals(SettingsPage.Assistant, SettingsRoutes.page("speech"))
        assertEquals(SettingsPage.Computers, SettingsRoutes.page("connectedtools"))
    }

    @Test fun everyOldSettingsLinkStillResolvesAndParses() {
        val old = listOf("home", "safety", "keys", "messaging", "devices", "spending", "about", "background", "keeprunning", "notifications", "origins", "connectedtools", "computers", "capturewidgets", "storedkeys", "update", "speech", "assistant", "calendar", "contacts", "suggests")
        old.forEach { name ->
            assertNotNull("$name resolves", SettingsRoutes.stack(name))
            assertEquals("$name parses as a link", name, Links.parse("hq://settings/$name")?.screen)
        }
    }

    @Test fun theNewNamesWork() {
        listOf("backup", "updates", "tools").forEach { assertNotNull(it, SettingsRoutes.stack(it)) }
        assertEquals(SettingsPage.About, SettingsRoutes.page("backup"))
        assertEquals(SettingsPage.Update, SettingsRoutes.page("updates"))
        assertEquals(SettingsPage.Update, SettingsRoutes.page("update"))
        assertEquals(SettingsPage.About, SettingsRoutes.page("about"))
        assertEquals(SettingsPage.Keys, SettingsRoutes.page("keys"))
    }

    @Test fun aNameIsReadWithoutRegardToCase() = assertEquals(SettingsPage.KeepRunning, SettingsRoutes.page("KeepRunning"))

    @Test fun keepRunningOpensWithBackgroundWorkUnderIt() {
        assertEquals(listOf(SettingsPage.Background, SettingsPage.KeepRunning), SettingsRoutes.stack("keeprunning"))
        assertEquals(listOf(SettingsPage.Safety, SettingsPage.Origins), SettingsRoutes.stack("origins"))
        assertEquals(emptyList<SettingsPage>(), SettingsRoutes.stack("home"))
        assertEquals(listOf(SettingsPage.Keys), SettingsRoutes.stack("keys"))
        assertNull(SettingsRoutes.stack("nope"))
    }

    @Test fun aPairingOrSetupPageHasNoLink() {
        assertNull(Links.parse("hq://settings/setupapp"))
        assertNull(Links.parse("hq://settings/pairapp"))
    }

    @Test fun everyStateLineIsFortyCharactersOrFewerWhateverHqSays() {
        val settings = JSONObject("""{"provider_keys":[{"name":"OpenRouter","configured":true}],"agent_host":{"hosts":["mac-mini","native"]}}""")
        val chat = JSONObject("""{"telegram":{"enabled":true,"token_set":true},"discord":{"enabled":false}}""")
        val github = JSONObject("""{"standing":"connected","owner":"example-owner"}""")
        val snaps = listOf(SettingsSnapshot(), SettingsSnapshot(settings = settings, chatApps = chat, github = github), SettingsSnapshot(github = JSONObject("""{"standing":"needs_install"}""")))
        snaps.forEach { snap ->
            val long = listOf(HomeRow(SettingsPage.Background, "background", "Background work", "x".repeat(80)))
            SettingsWords.home(testWords(), snap, now, long, loaded = true, updates = "y".repeat(90) to false).flatMap { it.rows }.forEach {
                assertTrue("${it.title}: ${it.status}", it.status.length <= SettingsWords.STATE_MAX)
            }
        }
    }

    @Test fun backgroundWorkCarriesWhatKeepRunningNeedsFixed() {
        val w = testWords()
        assertEquals("On · 1 to fix", SettingsWords.backgroundHome(w, true, 1))
        assertEquals("Only when I open the app · 2 to fix", SettingsWords.backgroundHome(w, false, 2))
        assertEquals(SettingsWords.backgroundWords(w, true), SettingsWords.backgroundHome(w, true, 0))
    }

    @Test fun computersAndToolsIsOneRowWithItsCount() {
        val w = testWords()
        val two = home(SettingsSnapshot(settings = JSONObject("""{"agent_host":{"hosts":["a","b"]}}"""))).flatMap { it.rows }.first { it.page == SettingsPage.Computers }
        assertEquals("Computers and tools", two.title)
        assertEquals("2 computers paired", two.status)
        val none = home(SettingsSnapshot(settings = JSONObject("""{}"""))).flatMap { it.rows }.first { it.page == SettingsPage.Computers }
        assertEquals("No computer paired", none.status)
        assertEquals("Not read yet", SettingsWords.home(w, SettingsSnapshot(), now, phoneRows(), loaded = false).flatMap { it.rows }.first { it.page == SettingsPage.Computers }.status)
    }

    @Test fun rowsAndPageTitlesAgreeForTheFiveKnownMismatches() {
        val rows = home().flatMap { it.rows }.associate { it.page to it.title }
        assertEquals("Backup and moving phones", rows[SettingsPage.About])
        assertEquals("Updates", rows[SettingsPage.Update])
        assertEquals("Computers and tools", rows[SettingsPage.Computers])
        val w = testWords()
        assertEquals("Allowed addresses", w.s(example.hq.spike.R.string.settings_origins_count))
        assertEquals("Allowed addresses", w.s(example.hq.spike.R.string.settings_page_origins))
        assertEquals("Calendar", w.s(example.hq.spike.R.string.cal_row_title))
        assertEquals("Computers and tools", w.s(example.hq.spike.R.string.settings_page_computers))
        assertEquals("Updates", w.s(example.hq.spike.R.string.upd_page_title))
    }

    @Test fun updatesSaysReadyOrAVersionAndWarnsOnlyWhenSomethingIsWrong() {
        val w = testWords()
        assertEquals("Ready · 1.2.0" to false, SettingsWords.updatesHome(w, UpdateState.Idle, "1.1.0", "1.2.0"))
        assertEquals("Version 1.1.0" to false, SettingsWords.updatesHome(w, UpdateState.UpToDate, "1.1.0", null))
        assertEquals("Needs a look" to true, SettingsWords.updatesHome(w, UpdateState.Failed("x"), "1.1.0", null))
        assertEquals("Needs a look" to true, SettingsWords.updatesHome(w, UpdateState.InstalledWithdrawn("1.1.0", null), "1.1.0", null))
    }

    @Test fun homeNeverStacksCardsAndTheUpdateCardYieldsToAnyWarning() {
        assertEquals(SettingsWords.HomeCard.Withdrawn, SettingsWords.homeCard(true, 2, true, true))
        assertEquals(SettingsWords.HomeCard.KeepStops, SettingsWords.homeCard(false, 1, true, true))
        assertEquals(SettingsWords.HomeCard.Backup, SettingsWords.homeCard(false, 0, true, true))
        assertEquals(SettingsWords.HomeCard.Update, SettingsWords.homeCard(false, 0, false, true))
        assertEquals(SettingsWords.HomeCard.None, SettingsWords.homeCard(false, 0, false, false))
    }

    @Test fun homeHasNineteenRowsAndSearchIsNotBuilt() {
        // The board counts 17 rows. What HQ suggests and Contacts shipped after it and have no other home, so Home has 19: over 15, and search is still not built.
        assertEquals(19, home().sumOf { it.rows.size })
    }
}
