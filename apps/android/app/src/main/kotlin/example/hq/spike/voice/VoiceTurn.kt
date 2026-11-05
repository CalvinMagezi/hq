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

/** What one spoken request produces, in order: the thread it landed in, the reply as it streams, then one ending. */
sealed interface TurnOutput {
    data class Thread(val id: String) : TurnOutput
    data class Text(val chunk: String) : TurnOutput
    /** HQ is looking something up before it answers; shown so the owner knows why nothing has appeared yet. */
    data object Working : TurnOutput
    data object Done : TurnOutput
    data class Failed(val reason: String) : TurnOutput
    /** The owner's OK is needed before HQ can finish; the turn is over for now, the sheet says so. */
    data object NeedsOk : TurnOutput
    /** Events were missed and the reply was read again from HQ: what is shown so far is replaced by this. */
    data class Reset(val text: String) : TurnOutput
}

/**
 * Sends one transcript to HQ's chat and passes on its streamed reply. The socket carries every chat's
 * events, so this turn knows its own by the client id it sent and then by the thread HQ put it in. Every path ends:
 * a reply, an error, a refusal, the socket dropping, missed events, or too long a silence.
 */
class VoiceTurn(
    private val events: SharedFlow<ChatEvent>,
    private val connected: StateFlow<Boolean>,
    private val post: (content: String, threadId: String?, clientId: String) -> Boolean,
    private val startWaitMs: Long = START_WAIT_MS,
    private val quietWaitMs: Long = QUIET_WAIT_MS,
    /** Asks the server to say where a thread's reply stands (`resume`). */
    private val resume: (threadId: String) -> Unit = {},
    /** The last saved reply of a thread, or null when there is none yet. */
    private val lastReply: suspend (threadId: String) -> String? = { null },
    /** Whether HQ still reports the thread as running; a silent turn that is still running is not given up on. */
    private val stillRunning: suspend (threadId: String) -> Boolean = { false },
) {
    /** Cold: the frame goes out only once this turn is listening, so the first reply event cannot be missed. */
    fun ask(transcript: String, threadId: String?, clientId: String = newClientId()): Flow<TurnOutput> = channelFlow {
        val listening = CompletableDeferred<Unit>()
        val lastHeard = AtomicLong(System.currentTimeMillis())
        val started = AtomicBoolean(false)
        val resuming = AtomicBoolean(false)
        val threadOfTurn = java.util.concurrent.atomic.AtomicReference<String?>(threadId)
        // Whoever ends the turn first (the reply itself or a watcher) is the only ending sent.
        val ended = AtomicBoolean(false)
        val reading = launch {
            var thread: String? = null
            // Signalled once, when HQ starts a tool before any of the answer: a lookup can take many seconds.
            var answered = false
            events.onSubscription { listening.complete(Unit) }.transformWhile { event ->
                if (event is ChatEvent.Lagged && thread != null) {
                    resuming.set(true)
                    resume(thread!!)
                    return@transformWhile true
                }
                if (event is ChatEvent.ReplySoFar && resuming.get() && event.threadId == thread) {
                    resuming.set(false)
                    lastHeard.set(System.currentTimeMillis())
                    if (event.running) {
                        emit(TurnOutput.Reset(event.content))
                        return@transformWhile true
                    }
                    if (!ended.compareAndSet(false, true)) return@transformWhile false
                    emit(TurnOutput.Reset(lastReply(thread!!) ?: ""))
                    emit(TurnOutput.Done)
                    return@transformWhile false
                }
                val step = step(event, clientId, thread)
                if (step.ours) lastHeard.set(System.currentTimeMillis())
                if (thread == null && step.thread != null) {
                    thread = step.thread
                    started.set(true)
                    threadOfTurn.set(step.thread)
                    emit(TurnOutput.Thread(step.thread))
                }
                if (step.ends && !ended.compareAndSet(false, true)) return@transformWhile false
                if (!answered && event is ChatEvent.ToolStarted && step.ours) {
                    answered = true
                    emit(TurnOutput.Working)
                }
                step.out.forEach {
                    if (it is TurnOutput.Text) answered = true
                    emit(it)
                }
                !step.ends
            }.collect { send(it) }
        }
        listening.await()
        if (!post(VOICE_CONTEXT + transcript, threadId, clientId)) {
            reading.cancel()
            send(TurnOutput.Failed(HQ_STOPPED))
            return@channelFlow
        }
        suspend fun endWith(reason: String) {
            if (!reading.isActive || !ended.compareAndSet(false, true)) return
            reading.cancel()
            send(TurnOutput.Failed(reason))
        }
        val watching = launch {
            launch {
                // A frame can be queued before the socket finishes opening; only a drop after it opened counts.
                connected.first { it }
                while (reading.isActive) {
                    connected.first { !it }
                    val id = threadOfTurn.get()
                    // HQ itself is still working: wait for the socket and ask where the reply stands, rather than give up.
                    if (id != null && stillRunning(id)) {
                        connected.first { it }
                        lastHeard.set(System.currentTimeMillis())
                        resuming.set(true)
                        resume(id)
                    } else {
                        endWith(RESTARTING)
                        break
                    }
                }
            }
            while (reading.isActive) {
                delay(WATCH_TICK_MS)
                val limit = if (started.get()) quietWaitMs else startWaitMs
                if (System.currentTimeMillis() - lastHeard.get() > limit) {
                    val id = threadOfTurn.get()
                    if (started.get() && id != null && stillRunning(id)) {
                        lastHeard.set(System.currentTimeMillis())
                    } else {
                        endWith(if (started.get()) STILL_WORKING else NO_REPLY)
                    }
                }
            }
        }
        reading.join()
        watching.cancel()
    }

    private class Step(val out: List<TurnOutput> = emptyList(), val thread: String? = null, val ends: Boolean = false, val ours: Boolean = false)

    private fun step(event: ChatEvent, clientId: String, thread: String?): Step {
        if (event is ChatEvent.TurnStart) return if (event.clientId == clientId) Step(thread = event.threadId, ours = true) else Step()
        if (event is ChatEvent.Rejected) {
            if (event.clientId != clientId) return Step()
            val failed = TurnOutput.Failed(rejected(event))
            return if (event.running && !event.threadId.isNullOrBlank()) Step(listOf(failed), thread = event.threadId, ends = true) else Step(listOf(failed), ends = true)
        }
        // Events were dropped before this turn had a thread to ask about; nothing can be read back.
        if (event is ChatEvent.Lagged) return Step(listOf(TurnOutput.Failed(LOST_PART)), ends = true)
        if (thread == null || threadOf(event) != thread) return Step()
        return when (event) {
            is ChatEvent.Text -> Step(listOf(TurnOutput.Text(event.text)), ours = true)
            is ChatEvent.Failed -> Step(listOf(TurnOutput.Failed(event.message)), ends = true)
            is ChatEvent.TurnEnd -> Step(listOf(if (event.waitingForOwner) TurnOutput.NeedsOk else TurnOutput.Done), ends = true)
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
        fun newClientId() = CLIENT_PREFIX + UUID.randomUUID()

        /**
         * Sent ahead of every spoken question. HQ passes an <hq-context> block to the model and leaves it out of the
         * saved chat, so the thread shows only what the owner said.
         */
        const val VOICE_CONTEXT = "<hq-context>The owner asked this by voice and reads the answer on a small sheet " +
            "over whatever they were doing. Answer in one to three short sentences of plain text: no markdown, lists, " +
            "tables or code. " +
            "Use a tool only when the answer needs the owner's own data or an action. " +
            "If more detail would help, offer it in one sentence instead of giving it.</hq-context>\n\n"
        /** HQ saves the question and says so within milliseconds; this long with no word means it never arrived. */
        const val START_WAIT_MS = 20_000L
        /** Tools can run for a while, but each step reports; this long with nothing at all is a stuck turn. */
        const val QUIET_WAIT_MS = 180_000L
        private const val WATCH_TICK_MS = 500L
        /** Failure codes, not sentences: the sheet turns each into its catalogue line. */
        const val HQ_STOPPED = "hq-turn:hq_stopped"
        const val RESTARTING = "hq-turn:restarting"
        const val NO_REPLY = "hq-turn:no_reply"
        const val STILL_WORKING = "hq-turn:still_working"
        const val LOST_PART = "hq-turn:lost_part"
        const val BUSY = "hq-turn:busy"
        const val NOT_ACCEPTED = "hq-turn:not_accepted"
    }
}
