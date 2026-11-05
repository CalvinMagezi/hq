package example.hq.spike.ui.shell

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.Text
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w390dp-h800dp-xxhdpi")
class ReselectEffectTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private fun show(bus: ReselectBus, atTop: (Place) -> Unit, rows: Int = 100, readsAgain: Boolean = true): androidx.compose.foundation.lazy.LazyListState {
        lateinit var state: androidx.compose.foundation.lazy.LazyListState
        rule.setContent {
            state = rememberLazyListState()
            CompositionLocalProvider(LocalReselect provides bus.taps, LocalReselectAtTop provides atTop) {
                ReselectEffect(Place.Tasks, state, readsAgain)
                LazyColumn(Modifier.height(300.dp), state = state) { items((0 until rows).toList()) { Text("row $it", Modifier.height(48.dp)) } }
            }
        }
        return state
    }

    @Test fun aSecondTapScrollsToTheTopThenAskForARefreshAtTheTop() {
        val bus = ReselectBus()
        var refreshed = 0
        val state = show(bus, { refreshed++ })
        runBlocking { rule.runOnUiThread { }; }
        rule.runOnIdle { runBlocking { state.scrollToItem(10) } }
        rule.waitForIdle()
        bus.tap(Place.Tasks)
        rule.waitUntil(5_000) { state.firstVisibleItemIndex == 0 }
        assertEquals(0, refreshed)
        bus.tap(Place.Tasks)
        rule.waitUntil(5_000) { refreshed == 1 }
    }

    @Test fun aTapOnAnotherPlaceDoesNothingHere() {
        val bus = ReselectBus()
        var refreshed = 0
        show(bus, { refreshed++ })
        bus.tap(Place.Notes)
        rule.waitForIdle()
        assertEquals(0, refreshed)
    }

    @Test fun aChatListHasNothingToReadAgain() {
        val bus = ReselectBus()
        var refreshed = 0
        show(bus, { refreshed++ }, readsAgain = false)
        bus.tap(Place.Tasks)
        rule.waitForIdle()
        assertFalse(refreshed > 0)
    }
}
