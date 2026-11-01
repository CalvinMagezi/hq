package example.hq.spike.voice

import example.hq.spike.data.ChatEvent
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.channelFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.onSubscription
import kotlinx.coroutines.flow.transformWhile
import kotlinx.coroutines.launch

/** What one spoken request produces, in order: the thread it landed in, sentences to say, then one ending. */
sealed interface TurnOutput {
    data class Thread(val id: String) : TurnOutput
    data class Say(val sentence: String) : TurnOutput
    data object Done : TurnOutput
    data class Failed(val reason: String) : TurnOutput
}

/**
 * Sends one transcript to HQ's chat and turns the streamed reply into sentences. The socket carries every chat's
 * events, so this turn knows its own by the client id it sent and then by the thread HQ put it in. Every path ends:
 * a reply, an error, a refusal, the socket dropping, missed events, or too long a silence.
 */
class VoiceTurn(
    private val events: SharedFlow<ChatEvent>,
    private val connected: StateFlow<Boolean>,
    private val post: (content: String, threadId: String?, clientId: String) -> Boolean,
    private val startWaitMs: Long = START_WAIT_MS,
    private val quietWaitMs: Long = QUIET_WAIT_MS,
) {
    /** Cold: the frame goes out only once this turn is listening, so the first reply event cannot be missed. */
    fun ask(transcript: String, threadId: String?): Flow<TurnOutput> = channelFlow {
        val clientId = CLIENT_PREFIX + UUID.randomUUID()
        val listening = CompletableDeferred<Unit>()
        val lastHeard = AtomicLong(System.currentTimeMillis())
        val started = AtomicBoolean(false)
        val reading = launch {
            val words = SpokenText()
            var thread: String? = null
            events.onSubscription { listening.complete(Unit) }.transformWhile { event ->
                val step = step(event, clientId, thread, words)
                if (step.ours) lastHeard.set(System.currentTimeMillis())
                if (thread == null && step.thread != null) {
                    thread = step.thread
                    started.set(true)
                    emit(TurnOutput.Thread(step.thread))
                }
                step.out.forEach { emit(it) }
                !step.ends
            }.collect { send(it) }
        }
        listening.await()
        if (!post(transcript, threadId, clientId)) {
            reading.cancel()
            send(TurnOutput.Failed(NOT_CONNECTED))
            return@channelFlow
        }
        // The first watcher to fire ends the turn; the reader's own ending wins if it came first.
        val ended = AtomicBoolean(false)
        suspend fun endWith(reason: String) {
            if (!reading.isActive || !ended.compareAndSet(false, true)) return
            reading.cancel()
            send(TurnOutput.Failed(reason))
        }
        val watching = launch {
            launch {
                connected.first { !it }
                endWith(CONNECTION_LOST)
            }
            while (reading.isActive) {
                delay(WATCH_TICK_MS)
                val limit = if (started.get()) quietWaitMs else startWaitMs
                if (System.currentTimeMillis() - lastHeard.get() > limit) endWith(if (started.get()) TOO_QUIET else NOT_STARTED)
            }
        }
        reading.join()
        watching.cancel()
    }

    private class Step(val out: List<TurnOutput> = emptyList(), val thread: String? = null, val ends: Boolean = false, val ours: Boolean = false)

    private fun step(event: ChatEvent, clientId: String, thread: String?, words: SpokenText): Step {
        if (event is ChatEvent.TurnStart) return if (event.clientId == clientId) Step(thread = event.threadId, ours = true) else Step()
        if (event is ChatEvent.Rejected) {
            return if (event.clientId == clientId) Step(listOf(TurnOutput.Failed(rejected(event))), ends = true) else Step()
        }
        // Events were dropped for a slow reader; this turn's end may be among them.
        if (event is ChatEvent.Lagged) return Step(listOf(TurnOutput.Failed(MISSED)), ends = true)
        if (thread == null || threadOf(event) != thread) return Step()
        return when (event) {
            is ChatEvent.Text -> Step(words.add(event.text).map(TurnOutput::Say), ours = true)
            is ChatEvent.Failed -> Step(listOf(TurnOutput.Failed(event.message)), ends = true)
            is ChatEvent.TurnEnd -> Step(words.finish().map(TurnOutput::Say) + TurnOutput.Done, ends = true)
            else -> Step(ours = true)
        }
    }

    private fun threadOf(event: ChatEvent): String? = when (event) {
        is ChatEvent.Text -> event.threadId
        is ChatEvent.Failed -> event.threadId
        is ChatEvent.TurnEnd -> event.threadId
        is ChatEvent.Reasoning -> event.threadId
        is ChatEvent.ToolStarted -> event.threadId
        is ChatEvent.ToolProgress -> event.threadId
        is ChatEvent.ToolFinished -> event.threadId
        else -> null
    }

    private fun rejected(event: ChatEvent.Rejected) = if (event.running) BUSY else event.reason.ifBlank { NOT_ACCEPTED }

    companion object {
        const val CLIENT_PREFIX = "voice-"
        /** HQ saves the question and says so within milliseconds; this long with no word means it never arrived. */
        const val START_WAIT_MS = 20_000L
        /** Tools can run for a while, but each step reports; this long with nothing at all is a stuck turn. */
        const val QUIET_WAIT_MS = 180_000L
        private const val WATCH_TICK_MS = 500L
        const val NOT_CONNECTED = "HQ is not answering yet. Try again in a moment."
        const val CONNECTION_LOST = "The connection to HQ dropped before the answer finished."
        const val NOT_STARTED = "HQ did not pick up the question. Try again in a moment."
        const val TOO_QUIET = "HQ went quiet in the middle of the answer. It may still finish in the chat."
        const val MISSED = "Part of the answer was missed on the way. The full answer is in the chat."
        const val BUSY = "HQ is still answering the last question."
        const val NOT_ACCEPTED = "HQ did not accept the question."
    }
}
