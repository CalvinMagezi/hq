package example.hq.spike.ui.settings

import android.app.Application
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasImeAction
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import example.hq.spike.testing.testWords
import example.hq.spike.ui.shell.Links
import example.hq.spike.ui.theme.HqTheme
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runTest
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w390dp-h1200dp-xxhdpi")
class SettingsSearchTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Before fun english() = java.util.Locale.setDefault(java.util.Locale.ENGLISH)

    private fun index() = SettingsIndex.build(testWords())

    @Test fun emptyOrBlankQueryReturnsNoResults() {
        val idx = index()
        assertTrue(SettingsSearch.results("", idx).isEmpty())
        assertTrue(SettingsSearch.results("   ", idx).isEmpty())
    }

    @Test fun matchesPageNamesSectionsAndAliasesByWordPrefixCaseInsensitively() {
        val w = testWords()
        val idx = index()

        val speech = SettingsSearch.results("SPEECH", idx).map { SettingsIndex.rowLabel(w, it) }
        assertTrue("Expected Speech model result, got $speech", "Phone assistant, Speech model" in speech)

        val storedKeys = SettingsSearch.results("storedkeys", idx).map { SettingsIndex.rowLabel(w, it) }
        assertTrue("Expected Keys on this phone result, got $storedKeys", "AI service and keys, Keys on this phone" in storedKeys)

        val backup = SettingsSearch.results("backup", idx).map { SettingsIndex.rowLabel(w, it) }
        assertTrue("Expected Backup result, got $backup", "Backup and moving phones" in backup)

        val androidStops = SettingsSearch.results("android stop", idx).map { SettingsIndex.rowLabel(w, it) }
        assertTrue("Expected Background work section, got $androidStops", "Background work, If Android stops HQ" in androidStops)

        val origins = SettingsSearch.results("allowed", idx).map { SettingsIndex.rowLabel(w, it) }
        assertTrue("Expected Allowed addresses result, got $origins", "Safety and limits, Allowed addresses" in origins)

        val tools = SettingsSearch.results("tools", idx).map { SettingsIndex.rowLabel(w, it) }
        assertTrue("Expected Connected tools result, got $tools", "Computers and tools, Connected tools" in tools)

        assertTrue(SettingsSearch.results("dark", idx).isEmpty())
    }

    @Test fun resultsAreCappedAtEight() {
        val results = SettingsSearch.results("s", index())
        assertTrue("Expected <= 8 results, got ${results.size}", results.size <= SettingsSearch.MAX_RESULTS)
        assertEquals(8, results.size)
    }

    @Test fun aliasesResolveToExpectedPageAndSection() {
        assertEquals(SettingsRoutes.RouteTarget(listOf(SettingsPage.Keys), "storedkeys"), SettingsRoutes.resolve("storedkeys"))
        assertEquals(SettingsRoutes.RouteTarget(listOf(SettingsPage.Assistant), "speech"), SettingsRoutes.resolve("speech"))
        assertEquals(SettingsRoutes.RouteTarget(listOf(SettingsPage.Computers), "connectedtools"), SettingsRoutes.resolve("connectedtools"))
        assertEquals(SettingsRoutes.RouteTarget(listOf(SettingsPage.Computers), "connectedtools"), SettingsRoutes.resolve("tools"))
        assertEquals(SettingsRoutes.RouteTarget(listOf(SettingsPage.About), null), SettingsRoutes.resolve("backup"))
        assertEquals(SettingsRoutes.RouteTarget(listOf(SettingsPage.Update), null), SettingsRoutes.resolve("updates"))
        assertEquals(SettingsRoutes.RouteTarget(listOf(SettingsPage.Background, SettingsPage.KeepRunning), null), SettingsRoutes.resolve("keeprunning"))
        assertEquals(SettingsRoutes.RouteTarget(listOf(SettingsPage.Safety, SettingsPage.Origins), null), SettingsRoutes.resolve("origins"))
        assertNull(SettingsRoutes.resolve("nope"))

        listOf("storedkeys", "speech", "connectedtools", "tools", "backup", "updates", "keeprunning", "origins").forEach { alias ->
            val link = Links.parse("hq://settings/$alias")
            assertEquals(alias, link?.screen)
            assertEquals(SettingsRoutes.resolve(alias), SettingsRoutes.resolve(link!!.screen!!))
        }
    }

    @Test fun scrollTargetMapsKnownSectionsAndFallsBackToTop() {
        assertEquals(ScrollTarget.AssistantSpeech, SettingsScroll.target(SettingsPage.Assistant, "speech"))
        assertEquals(ScrollTarget.KeysStored, SettingsScroll.target(SettingsPage.Keys, "storedkeys"))
        assertEquals(ScrollTarget.BackgroundKeepRunning, SettingsScroll.target(SettingsPage.Background, "keeprunning"))
        assertEquals(ScrollTarget.SafetyAllowedAddresses, SettingsScroll.target(SettingsPage.Safety, "origins"))
        assertEquals(ScrollTarget.SafetyTrustedSites, SettingsScroll.target(SettingsPage.Safety, "trusted_sites"))
        assertEquals(ScrollTarget.ComputersTools, SettingsScroll.target(SettingsPage.Computers, "connectedtools"))

        assertEquals(ScrollTarget.Top, SettingsScroll.target(SettingsPage.Keys, null))
        assertEquals(ScrollTarget.Top, SettingsScroll.target(SettingsPage.Keys, ""))
        assertEquals(ScrollTarget.Top, SettingsScroll.target(SettingsPage.Keys, "unknown"))
        assertEquals(ScrollTarget.Top, SettingsScroll.target(SettingsPage.Keys, "speech"))
    }

    @Test fun openingAnySafetySectionForcesSafetyTab() {
        val vm = SettingsViewModel(ApplicationProvider.getApplicationContext<Application>())
        vm.actions.onTab(SettingsWords.SafetyTab.Time)
        assertEquals(SettingsWords.SafetyTab.Time, vm.state.value.safetyTab)

        vm.actions.onPageSection(SettingsPage.Safety, "trusted_sites")
        assertEquals(SettingsWords.SafetyTab.Safety, vm.state.value.safetyTab)

        vm.actions.onTab(SettingsWords.SafetyTab.Helpers)
        vm.actions.onPageSection(SettingsPage.Safety, "origins")
        assertEquals(SettingsWords.SafetyTab.Safety, vm.state.value.safetyTab)
    }

    @Test fun scrollOffsetSubtractsRootTopEvenWhenRootTopArrivesLate() {
        assertEquals(420, SettingsScroll.offset(600f, 180))
        assertEquals(0, SettingsScroll.offset(100f, 180))

        val machine = SectionScrollMachine(ScrollTarget.KeysStored, reducedMotion = true)
        machine.register(ScrollTarget.KeysStored, 600f)
        assertNull(machine.targetOffset())
        val tickBefore = machine.layoutTick
        machine.setRootTop(180)
        assertEquals(420, machine.targetOffset())
        assertTrue(machine.layoutTick > tickBefore)
    }

    @Test fun sectionScrollMachineHandlesStartAbortRestartAndFinish() = runTest {
        // 1. Abort during scrollTo: target is not marked done, so a restart runs again and finishes.
        val m1 = SectionScrollMachine(ScrollTarget.KeysStored, reducedMotion = false)
        m1.setRootTop(100)
        m1.register(ScrollTarget.KeysStored, 500f)
        val scrollStarted = CompletableDeferred<Unit>()
        val scrollGate = CompletableDeferred<Unit>()
        var scrollCalls = 0
        var consumed = 0
        val job1 = launch {
            m1.runIfReady(
                scrollTo = { scrollCalls++; scrollStarted.complete(Unit); scrollGate.await() },
                onConsumed = { consumed++ },
                waitHighlight = {},
            )
        }
        scrollStarted.await()
        job1.cancel()
        job1.join()
        assertNull("Aborted scroll must not mark target done", m1.scrolledTarget)
        assertNull("Aborted scroll must not leave highlight", m1.highlighted)
        assertEquals(0, consumed)

        m1.runIfReady(scrollTo = { scrollCalls++ }, onConsumed = { consumed++ }, waitHighlight = {})
        assertEquals(2, scrollCalls)
        assertEquals(ScrollTarget.KeysStored, m1.scrolledTarget)
        assertNull(m1.highlighted)
        assertEquals(1, consumed)

        // 2. Abort during highlight delay: NonCancellable finally clears highlight and consumes section.
        val m2 = SectionScrollMachine(ScrollTarget.AssistantSpeech, reducedMotion = false)
        m2.setRootTop(0)
        m2.register(ScrollTarget.AssistantSpeech, 300f)
        val waiting = CompletableDeferred<Unit>()
        val waitGate = CompletableDeferred<Unit>()
        var consumed2 = 0
        val job2 = launch {
            m2.runIfReady(
                scrollTo = {},
                onConsumed = { consumed2++ },
                waitHighlight = { waiting.complete(Unit); waitGate.await() },
            )
        }
        waiting.await()
        assertEquals(ScrollTarget.AssistantSpeech, m2.highlighted)
        job2.cancel()
        job2.join()
        assertNull("Cancelled delay must still clear highlight", m2.highlighted)
        assertEquals(1, consumed2)
    }

    @Test fun searchUiAnnouncesCountLabelsFieldExposesTextAndImeSearch() {
        var query by mutableStateOf("")
        var opened: Pair<SettingsPage, String?>? = null
        rule.setContent {
            HqTheme {
                SettingsHost(
                    SettingsUi(open = true, searchQuery = query, loaded = true),
                    SettingsActions(
                        onSearchQuery = { query = it },
                        onPageSection = { p, s -> opened = p to s },
                    ),
                )
            }
        }

        rule.onNodeWithContentDescription("Search settings").assertIsDisplayed()
        rule.onNode(hasImeAction(ImeAction.Search)).assertExists()
        rule.onNode(hasSetTextAction()).performTextInput("speech")
        rule.waitForIdle()
        assertEquals("speech", query)

        val fieldSemantics = rule.onNode(hasSetTextAction()).fetchSemanticsNode().config
        assertEquals("speech", fieldSemantics.getOrNull(SemanticsProperties.EditableText)?.text)
        assertNull("ContentDescription must not override EditableText", fieldSemantics.getOrNull(SemanticsProperties.ContentDescription))

        rule.onNodeWithText("Phone assistant, Speech model").assertIsDisplayed()
        val polite = rule.onAllNodes(hasText("1 result"), useUnmergedTree = false)
            .fetchSemanticsNodes()
            .any { it.config.getOrNull(SemanticsProperties.LiveRegion) != null }
        assertTrue("Expected polite liveRegion on result count", polite)

        val rowNode = rule.onAllNodes(hasContentDescription("Phone assistant, Speech model"), useUnmergedTree = false)
            .fetchSemanticsNodes()
        assertTrue("Expected result row description to include page name", rowNode.isNotEmpty())

        rule.onNodeWithText("Phone assistant, Speech model").performClick()
        assertEquals(SettingsPage.Assistant to "speech", opened)
    }

    @Test fun emptySearchResultShowsNothingMatchesPolitely() {
        rule.setContent {
            HqTheme {
                SettingsHost(SettingsUi(open = true, searchQuery = "dark", loaded = true), SettingsActions())
            }
        }
        rule.onNodeWithText("Nothing in Settings matches that.").assertIsDisplayed()
        val polite = rule.onAllNodes(hasText("Nothing in Settings matches that."), useUnmergedTree = false)
            .fetchSemanticsNodes()
            .any { it.config.getOrNull(SemanticsProperties.LiveRegion) != null }
        assertTrue(polite)
    }

    @Test fun openingBelowFoldSectionWithReducedMotionScrollsIntoViewAndConsumesSection() {
        Settings.Global.putFloat(rule.activity.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 0f)
        val settings = JSONObject("""{"model":{"active":"example/model","local_only":false},"provider_keys":[{"name":"OpenRouter","configured":true}]}""")
        var ui by mutableStateOf(SettingsUi(open = true, stack = listOf(SettingsPage.Keys), section = "storedkeys", snap = SettingsSnapshot(settings = settings), loaded = true))
        var consumedCount = 0
        rule.setContent {
            HqTheme {
                Box(Modifier.size(390.dp, 420.dp)) {
                    SettingsHost(ui, SettingsActions(onSectionConsumed = { consumedCount++; ui = ui.copy(section = null) }))
                }
            }
        }
        rule.waitForIdle()
        rule.mainClock.advanceTimeBy(1_500L)
        rule.waitForIdle()

        val bounds = rule.onNodeWithText("KEYS ON THIS PHONE").getUnclippedBoundsInRoot()
        assertTrue("Expected KEYS ON THIS PHONE scrolled into 420dp viewport, got $bounds", bounds.top.value >= 0f && bounds.bottom.value <= 420f)
        assertNull("Section should be consumed after scroll", ui.section)
        assertEquals(1, consumedCount)
    }

    @Test fun absentRuntimeSectionFallsBackToTopWithoutHighlight() {
        var ui by mutableStateOf(SettingsUi(open = true, stack = listOf(SettingsPage.Keys), section = "model", snap = SettingsSnapshot(settings = null), loaded = true))
        rule.setContent {
            HqTheme {
                Box(Modifier.size(390.dp, 420.dp)) {
                    SettingsHost(ui, SettingsActions(onSectionConsumed = { ui = ui.copy(section = null) }))
                }
            }
        }
        rule.waitForIdle()
        rule.onNodeWithText("KEYS ON THIS PHONE").assertExists()
    }

    @Test fun fieldAndResultRowFitAtTwoHundredPercentText() {
        val widthDp = 360
        rule.setContent {
            val d = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(d.density, 2.0f)) {
                HqTheme {
                    Box(Modifier.width(widthDp.dp)) {
                        Column {
                            SettingsSearchField(query = "", onQuery = {})
                            SettingsSearchResultRow(label = "AI service and keys, Keys on this phone", first = true, onClick = {})
                        }
                    }
                }
            }
        }
        rule.waitForIdle()
        val fieldBounds = rule.onNodeWithContentDescription("Search settings").getUnclippedBoundsInRoot()
        assertTrue("Search field cut off at 200%: $fieldBounds", fieldBounds.right.value <= widthDp + 1f)
        val rowBounds = rule.onNodeWithText("AI service and keys, Keys on this phone").getUnclippedBoundsInRoot()
        assertTrue("Result row cut off at 200%: $rowBounds", rowBounds.right.value <= widthDp + 1f)
    }
}
