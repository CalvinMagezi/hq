package example.hq.spike.ui.inbox

import example.hq.spike.data.HqUnavailable
import example.hq.spike.ui.shell.UndoResult
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** What one Undo put back: the cards to show again, and whether HQ refused any restore as too late. */
class Undone(val cards: List<InboxCard>, val late: Boolean)

/**
 * Clears run one after another, so a second clear never cuts the first short, and each keeps its own record of what HQ
 * actually dismissed. Undo restores exactly that, newest batch first, and stops a batch that has not finished.
 */
class ClearQueue(private val dismiss: suspend (String) -> Unit, private val restore: suspend (String) -> UndoResult) {
    class Batch(val cards: List<InboxCard>) {
        val dismissed = mutableListOf<String>()
        var stopped = false
    }

    private val mutex = Mutex()
    private val batches = ArrayDeque<Batch>()

    /** Starts a batch; call [run] with it to dismiss. */
    fun begin(cards: List<InboxCard>): Batch = Batch(cards).also { batches.addLast(it) }

    /** True while [batch] can still be undone. */
    fun latest(): Batch? = batches.lastOrNull()

    suspend fun run(batch: Batch) = mutex.withLock {
        for (id in batch.cards.flatMap { it.ids }) {
            if (batch.stopped) break
            try { dismiss(id); batch.dismissed += id } catch (e: HqUnavailable) { /* it shows again on the next refresh */ }
        }
    }

    /** The undo window of [batch] ended: it can no longer be undone. */
    fun expire(batch: Batch) { batches.remove(batch) }

    suspend fun undo(): Undone? {
        val batch = batches.removeLastOrNull() ?: return null
        batch.stopped = true
        mutex.withLock { /* the batch's loop, if it was running, has stopped after the id it was on */ }
        var late = false
        for (id in batch.dismissed.toList()) if (restore(id) == UndoResult.TooLate) late = true
        return Undone(batch.cards, late)
    }
}
