package example.hq.spike.ui.shell

import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.staticCompositionLocalOf
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.emptyFlow

/** The taps on a tab that is already selected, one place per tap (wave-4/01-offline.md, "Re-tapping a tab"). */
class ReselectBus {
    val taps = MutableSharedFlow<Place>(extraBufferCapacity = 4)
    fun tap(place: Place) { taps.tryEmit(place) }
}

val LocalReselect = staticCompositionLocalOf<Flow<Place>> { emptyFlow() }

/** What a second tap at the top does: read again while HQ answers, open the Connection sheet while it does not. */
val LocalReselectAtTop = staticCompositionLocalOf<(Place) -> Unit> { {} }

object Reselect {
    /** Rows between the top and a place where scrolling up is still animated; farther than this it jumps. */
    const val ANIMATE_ROWS = 30

    fun atTop(list: LazyListState): Boolean = list.firstVisibleItemIndex == 0 && list.firstVisibleItemScrollOffset == 0
}

/**
 * Scrolls [list] to its top when [place]'s tab is tapped again, and calls the shell's at-top action on a tap that finds it there.
 * A chat's list runs from the newest message, so its top is the newest message and there is nothing to read again.
 */
@Composable
fun ReselectEffect(place: Place, list: LazyListState, readsAgain: Boolean = true) {
    val taps = LocalReselect.current
    val atTop = LocalReselectAtTop.current
    LaunchedEffect(taps, place, list) {
        taps.collect { tapped ->
            if (tapped != place) return@collect
            when {
                !Reselect.atTop(list) -> if (list.firstVisibleItemIndex <= Reselect.ANIMATE_ROWS) list.animateScrollToItem(0) else list.scrollToItem(0)
                readsAgain -> atTop(place)
            }
        }
    }
}
