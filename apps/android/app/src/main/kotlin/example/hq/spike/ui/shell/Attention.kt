package example.hq.spike.ui.shell

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.platform.LocalContext
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import kotlinx.coroutines.delay

/** The four places the shell lets something ask for attention. Each shows at most one thing at a time. */
enum class AttentionSlot { Decision, Banner, Snackbar, Inline }

/**
 * Everything that can ask for attention, in authority order: approval, sign-in, blocking error, connection banner, undo
 * snackbar, helper question, offer, pills, digest. A lower position wins its slot. The banner kinds that are not the
 * connection sit right after it, so a connection problem always hides them. Approval, sign-in, blocking error, offer,
 * pills and digest have no poster yet; they hold their place in the order for the workstreams that add them.
 */
enum class AttentionKind(val slot: AttentionSlot) {
    Approval(AttentionSlot.Decision),
    SignIn(AttentionSlot.Decision),
    BlockingError(AttentionSlot.Decision),
    ConnectionBanner(AttentionSlot.Banner),
    NotificationsBlocked(AttentionSlot.Banner),
    UpdateLine(AttentionSlot.Banner),
    UndoSnackbar(AttentionSlot.Snackbar),
    Snackbar(AttentionSlot.Snackbar),
    HelperQuestion(AttentionSlot.Inline),
    Offer(AttentionSlot.Inline),
    Pills(AttentionSlot.Inline),
    Digest(AttentionSlot.Inline),
}

/**
 * One thing asking for attention. [kind] and [key] together are its identity: posting the same pair again replaces it in
 * place and restarts its timer ([token] counts the posts), never adds a second.
 * - [timeoutMs] overrides the kind's default time on screen.
 * - [onExpire] tells the poster the shell closed it (timer, or pushed out of the queue), so it can drop its own state.
 * - [announce] false keeps it out of the polite live region (a banner that swaps text when the place changes).
 * - [coversBar] is true when the poster sits on a full-screen page, so the snackbar is not lifted over a bar that is not there.
 * - [content] null means the poster draws itself in place (see [AttentionGate]).
 */
data class AttentionItem(
    val kind: AttentionKind,
    val key: String = "",
    val timeoutMs: Long? = null,
    val onExpire: (() -> Unit)? = null,
    val announce: Boolean = true,
    val coversBar: Boolean = false,
    val token: Int = 0,
    val content: (@Composable () -> Unit)? = null,
) {
    fun sameAs(other: AttentionItem) = kind == other.kind && key == other.key
}

/**
 * Rules R1 to R3 of the calm-UI report as plain state, with no Compose needed to test it: at most one decision, one
 * banner, one snackbar and one inline offer visible, the best-ranked kind wins a slot, a second snackbar waits behind
 * the first, and anything that ends is cleared so the next one shows.
 */
class AttentionBudget {
    private var all by mutableStateOf<List<AttentionItem>>(emptyList())

    /**
     * Adds [item], or replaces the one with the same kind and key in place and restarts its timer. Snackbars: one shows,
     * one waits. A newcomer pushes the waiting one out (its poster hears [AttentionItem.onExpire]), except that a plain
     * snackbar never pushes out an Undo: it is dropped instead.
     */
    fun post(item: AttentionItem) {
        val at = all.indexOfFirst { it.sameAs(item) }
        if (at >= 0) {
            all = all.toMutableList().also { it[at] = item.copy(token = all[at].token + 1) }
            return
        }
        var next = all
        if (item.kind.slot == AttentionSlot.Snackbar) {
            val waiting = next.filter { it.kind.slot == AttentionSlot.Snackbar }.getOrNull(QUEUE_DEPTH)
            if (waiting != null) {
                if (item.kind == AttentionKind.Snackbar && waiting.kind == AttentionKind.UndoSnackbar) {
                    item.onExpire?.invoke()
                    return
                }
                next = next - waiting
                waiting.onExpire?.invoke()
            }
        }
        all = next + item
    }

    /** Removes the item with this kind and key, if it is there. The poster ended it, so it is not told. */
    fun clear(kind: AttentionKind, key: String = "") {
        all = all.filterNot { it.kind == kind && it.key == key }
    }

    /** The shell closes [item] (its time is up): it goes, and its poster hears about it. */
    fun expire(item: AttentionItem) {
        if (all.none { it.sameAs(item) }) return
        clear(item.kind, item.key)
        item.onExpire?.invoke()
    }

    /** What shows in [slot] now, or null. Snackbars go in the order they came; other slots go to the best-ranked kind. */
    fun visible(slot: AttentionSlot): AttentionItem? {
        val inSlot = all.filter { it.kind.slot == slot }
        return if (slot == AttentionSlot.Snackbar) inSlot.firstOrNull() else inSlot.minByOrNull { it.kind.ordinal }
    }

    /** The snackbar waiting behind the visible one, if any. */
    fun queuedSnackbar(): AttentionItem? = all.filter { it.kind.slot == AttentionSlot.Snackbar }.getOrNull(QUEUE_DEPTH)

    /** Says [text] in a plain snackbar that closes by itself. Saying the same text again restarts its time. */
    fun say(text: String) = post(AttentionItem(AttentionKind.Snackbar, text) { SnackbarRow(text, null, {}) })

    /** Everything posted, for tests; the screen reads [visible]. */
    fun posted(): List<AttentionItem> = all

    companion object {
        /** A plain snackbar closes by itself after this long, counted from when it is shown. */
        const val PLAIN_SNACKBAR_MS = 4_000L

        /** An Undo snackbar stays this long (longer when the phone's accessibility timeout asks, see [Undo.millis]). */
        const val UNDO_SNACKBAR_MS = 10_000L

        /** Snackbars waiting behind the visible one. */
        const val QUEUE_DEPTH = 1

        /** How long the shell leaves [kind] up before it closes it, or null when it never times out (banners and decisions end with their cause). */
        fun timeoutMs(kind: AttentionKind): Long? = when (kind) {
            AttentionKind.UndoSnackbar -> UNDO_SNACKBAR_MS
            AttentionKind.Snackbar -> PLAIN_SNACKBAR_MS
            else -> null
        }

        /** What the shell uses for [item]: its own time, else the kind's; an Undo never shorter than the phone's accessibility timeout [phoneUndoMs]. */
        fun shownFor(item: AttentionItem, phoneUndoMs: Long): Long? {
            val base = item.timeoutMs ?: timeoutMs(item.kind) ?: return null
            return if (item.kind == AttentionKind.UndoSnackbar) maxOf(base, phoneUndoMs) else base
        }
    }
}

/** The shell's budget, or null where there is no shell (a screen on its own in a test): posters then draw themselves. */
val LocalAttention = staticCompositionLocalOf<AttentionBudget?> { null }

/** True under a full-screen page that hides the bottom bar, so a snackbar from it is not lifted over the bar. */
val LocalCoversBar = staticCompositionLocalOf { false }

/** Posts [content] while this call stays in the composition, and clears it when it leaves. */
@Composable
fun AttentionPost(
    kind: AttentionKind, key: String = "", timeoutMs: Long? = null, onExpire: (() -> Unit)? = null, announce: Boolean = true,
    restartOn: Any? = null, content: @Composable () -> Unit,
) {
    val budget = LocalAttention.current ?: return
    val covers = LocalCoversBar.current
    val latest by rememberUpdatedState(content)
    val latestExpire by rememberUpdatedState(onExpire)
    fun item() = AttentionItem(kind, key, timeoutMs, { latestExpire?.invoke() }, announce, covers) { latest() }
    DisposableEffect(budget, kind, key, covers) {
        budget.post(item())
        onDispose { budget.clear(kind, key) }
    }
    // A new value in [restartOn] (a new line in the same snackbar) starts its time again.
    var first by remember { mutableStateOf(true) }
    LaunchedEffect(restartOn) { if (first) first = false else budget.post(item()) }
}

/**
 * The poster keeps drawing itself in place (inside a list, say) but only while it is the one the budget lets show in its
 * slot; otherwise it draws nothing. Without a shell it always draws.
 */
@Composable
fun AttentionGate(kind: AttentionKind, key: String, content: @Composable () -> Unit) {
    val budget = LocalAttention.current
    if (budget == null) { content(); return }
    DisposableEffect(budget, kind, key) {
        budget.post(AttentionItem(kind, key))
        onDispose { budget.clear(kind, key) }
    }
    val shown = budget.visible(kind.slot)
    if (shown != null && shown.kind == kind && shown.key == key) content()
}

/** Draws [content] through the shell's budget, or in place with [modifier] when there is no shell. */
@Composable
fun AttentionOrInline(
    kind: AttentionKind, key: String, modifier: Modifier = Modifier, onExpire: (() -> Unit)? = null, announce: Boolean = true,
    restartOn: Any? = null, content: @Composable (Modifier) -> Unit,
) {
    if (LocalAttention.current == null) content(modifier) else AttentionPost(kind, key, null, onExpire, announce, restartOn) { content(Modifier) }
}

/** A key that stays the same for as long as one call site stays on screen, for posters that can overlap. */
@Composable
fun rememberAttentionKey(): String = remember { Any().hashCode().toString() }

/**
 * The one banner slot under the top bar. A banner that announces itself is a polite live region, read out when its
 * words change and without taking focus; one that swaps words only because the place changed ([AttentionItem.announce]
 * false) is not.
 */
@Composable
fun BannerSlot(budget: AttentionBudget) {
    val item = budget.visible(AttentionSlot.Banner) ?: return
    val live = if (item.announce) Modifier.semantics { liveRegion = LiveRegionMode.Polite } else Modifier
    Box(Modifier.fillMaxWidth().then(live)) { item.content?.invoke() }
}

/**
 * The one snackbar slot. The time counts from when an item is shown here, not from when its poster composed it. A snackbar
 * from a full-screen page sits at [belowBar] from the bottom edge, one from the shell at [aboveBar], clear of the bar.
 */
@Composable
fun SnackbarSlot(budget: AttentionBudget, aboveBar: Dp, belowBar: Dp, modifier: Modifier = Modifier) {
    val item = budget.visible(AttentionSlot.Snackbar) ?: return
    val context = LocalContext.current
    LaunchedEffect(item.kind, item.key, item.token) {
        AttentionBudget.shownFor(item, Undo.millis(context))?.let { delay(it); budget.expire(item) }
    }
    Box(modifier.padding(bottom = if (item.coversBar) belowBar else aboveBar).fillMaxWidth().semantics { liveRegion = LiveRegionMode.Polite }) { item.content?.invoke() }
}
