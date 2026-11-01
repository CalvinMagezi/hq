package example.hq.spike.voice

import example.hq.spike.data.ChatEvent
import java.util.UUID
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.channelFlow
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
 * events, so this turn knows its own by the client id it sent and then by the thread HQ put it in.
 */
class VoiceTurn(
    private val events: SharedFlow<ChatEvent>,
    private val post: (content: String, threadId: String?, clientId: String) -> Boolean,
) {
    /** Cold: the frame goes out only once this turn is listening, so the first reply event cannot be missed. */
    fun ask(transcript: String, threadId: String?): Flow<TurnOutput> = channelFlow {
        val clientId = CLIENT_PREFIX + UUID.randomUUID()
        val listening = CompletableDeferred<Unit>()
        val reading = launch {
            val words = SpokenText()
            var thread: String? = null
            events.onSubscription { listening.complete(Unit) }.transformWhile { event ->
                val step = step(event, clientId, thread, words)
                if (thread == null && step.thread != null) {
                    thread = step.thread
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
        }
    }

    private class Step(val out: List<TurnOutput> = emptyList(), val thread: String? = null, val ends: Boolean = false)

    private fun step(event: ChatEvent, clientId: String, thread: String?, words: SpokenText): Step {
        if (event is ChatEvent.TurnStart) return if (event.clientId == clientId) Step(thread = event.threadId) else Step()
        if (event is ChatEvent.Rejected) {
            return if (event.clientId == clientId) Step(listOf(TurnOutput.Failed(rejected(event))), ends = true) else Step()
        }
        val ours = thread != null && threadOf(event) == thread
        if (!ours) return Step()
        return when (event) {
            is ChatEvent.Text -> Step(words.add(event.text).map(TurnOutput::Say))
            is ChatEvent.Failed -> Step(listOf(TurnOutput.Failed(event.message)), ends = true)
            is ChatEvent.TurnEnd -> Step(words.finish().map(TurnOutput::Say) + TurnOutput.Done, ends = true)
            else -> Step()
        }
    }

    private fun threadOf(event: ChatEvent): String? = when (event) {
        is ChatEvent.Text -> event.threadId
        is ChatEvent.Failed -> event.threadId
        is ChatEvent.TurnEnd -> event.threadId
        else -> null
    }

    private fun rejected(event: ChatEvent.Rejected) = if (event.running) BUSY else event.reason.ifBlank { NOT_ACCEPTED }

    companion object {
        const val CLIENT_PREFIX = "voice-"
        const val NOT_CONNECTED = "HQ is not answering yet. Try again in a moment."
        const val BUSY = "HQ is still answering the last question."
        const val NOT_ACCEPTED = "HQ did not accept the question."
    }
}
