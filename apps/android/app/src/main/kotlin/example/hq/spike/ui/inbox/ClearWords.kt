package example.hq.spike.ui.inbox

import example.hq.spike.R

/** The snackbar noun for a clear: updates, or helper requests, by what was cleared. */
object ClearWords {
    @androidx.annotation.PluralsRes
    fun plural(cards: List<InboxCard>): Int = if (cards.any { it.kind != CardKind.Update }) R.plurals.helpers_cleared else R.plurals.updates_cleared
}
