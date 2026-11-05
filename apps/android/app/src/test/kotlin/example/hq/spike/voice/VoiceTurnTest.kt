package example.hq.spike.voice

import example.hq.spike.data.ChatEvent
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Test

class VoiceTurnTest {
    private fun turn(
        events: MutableSharedFlow<ChatEvent>,
        sent: MutableList<String>,
        accept: Boolean = true,
        connected: MutableStateFlow<Boolean> = MutableStateFlow(true),
        startWaitMs: Long = VoiceTurn.START_WAIT_MS,
        script: (String) -> List<ChatEvent>,
    ) = VoiceTurn(events, connected, { content, _, clientId ->
        sent += content
        if (accept) script(clientId).forEach { check(events.tryEmit(it)) }
        accept
    }, startWaitMs = startWaitMs)

    private fun bus() = MutableSharedFlow<ChatEvent>(extraBufferCapacity = 64)

    @Test
    fun aDroppedSocketEndsTheTurn() = runBlocking {
        val events = bus()
        val connected = MutableStateFlow(true)
        val dropping = launch {
            delay(DROP_AFTER_MS)
            connected.value = false
        }
        val out = turn(events, mutableListOf(), connected = connected) { id ->
            listOf(ChatEvent.TurnStart("t3", id, null), ChatEvent.Text("t3", "Half an"))
        }.ask("hi", null).let { withTimeout(TIMEOUT_MS) { it.toList() } }
        dropping.join()
        assertEquals(TurnOutput.Failed(VoiceTurn.RESTARTING), out.last())
    }

    @Test
    fun aLookupBeforeTheAnswerIsAnnouncedOnce() = runBlocking {
        val events = bus()
        val step = example.hq.spike.data.ToolStep("c1", "task_list", null, null, null)
        val out = turn(events, mutableListOf()) { id ->
            listOf(
                ChatEvent.TurnStart("t6", id, null),
                ChatEvent.ToolStarted("t6", step),
                ChatEvent.ToolStarted("t6", step.copy(id = "c2")),
                ChatEvent.Text("t6", "Two tasks. "),
                ChatEvent.TurnEnd("t6", "m6", false),
            )
        }.ask("hi", null).let { withTimeout(TIMEOUT_MS) { it.toList() } }
        assertEquals(listOf(TurnOutput.Thread("t6"), TurnOutput.Working, TurnOutput.Text("Two tasks. "), TurnOutput.Done), out)
    }

    @Test
    fun aToolAfterTheAnswerStartedIsNotAnnounced() = runBlocking {
        val events = bus()
        val step = example.hq.spike.data.ToolStep("c1", "task_list", null, null, null)
        val out = turn(events, mutableListOf()) { id ->
            listOf(ChatEvent.TurnStart("t7", id, null), ChatEvent.Text("t7", "Sure. "), ChatEvent.ToolStarted("t7", step), ChatEvent.TurnEnd("t7", "m7", false))
        }.ask("hi", null).let { withTimeout(TIMEOUT_MS) { it.toList() } }
        assertEquals(listOf(TurnOutput.Thread("t7"), TurnOutput.Text("Sure. "), TurnOutput.Done), out)
    }

    @Test
    fun aSocketStillOpeningIsNotADrop() = runBlocking {
        val events = bus()
        val connected = MutableStateFlow(false)
        val out = turn(events, mutableListOf(), connected = connected) { id ->
            listOf(ChatEvent.TurnStart("t5", id, null), ChatEvent.Text("t5", "Fine. "), ChatEvent.TurnEnd("t5", "m5", false))
        }.ask("hi", null).let { withTimeout(TIMEOUT_MS) { it.toList() } }
        assertEquals(listOf(TurnOutput.Thread("t5"), TurnOutput.Text("Fine. "), TurnOutput.Done), out)
    }

    @Test
    fun missedEventsAreReadAgainFromTheThread() = runBlocking {
        val events = bus()
        val asked = mutableListOf<String>()
        val t = VoiceTurn(events, MutableStateFlow(true), { _, _, id ->
            events.tryEmit(ChatEvent.TurnStart("t4", id, null)); events.tryEmit(ChatEvent.Lagged(3)); true
        }, resume = { asked += it; events.tryEmit(ChatEvent.ReplySoFar("t4", false, "All done.", "m4")) }, lastReply = { "All done." })
        val out = withTimeout(TIMEOUT_MS) { t.ask("hi", null).toList() }
        assertEquals(listOf("t4"), asked)
        assertEquals(listOf(TurnOutput.Thread("t4"), TurnOutput.Reset("All done."), TurnOutput.Done), out)
    }

    @Test
    fun missedEventsBeforeAThreadStillFail() = runBlocking {
        val out = turn(bus(), mutableListOf()) { listOf(ChatEvent.Lagged(3)) }.ask("hi", null).let { withTimeout(TIMEOUT_MS) { it.toList() } }
        assertEquals(TurnOutput.Failed(VoiceTurn.LOST_PART), out.last())
    }

    @Test
    fun aTurnWaitingForTheOwnerSaysSo() = runBlocking {
        val out = turn(bus(), mutableListOf()) { id -> listOf(ChatEvent.TurnStart("t7", id, null), ChatEvent.TurnEnd("t7", "m7", false, true)) }
            .ask("hi", null).let { withTimeout(TIMEOUT_MS) { it.toList() } }
        assertEquals(TurnOutput.NeedsOk, out.last())
    }

    @Test
    fun aQuestionHqNeverPicksUpTimesOut() = runBlocking {
        val out = turn(bus(), mutableListOf(), startWaitMs = 50) { emptyList() }.ask("hi", null).let { withTimeout(TIMEOUT_MS) { it.toList() } }
        assertEquals(listOf(TurnOutput.Failed(VoiceTurn.NO_REPLY)), out)
    }

    @Test
    fun aReplyStreamsThenEnds() = runBlocking {
        val events = bus()
        val sent = mutableListOf<String>()
        val out = turn(events, sent) { id ->
            listOf(
                ChatEvent.Text("other", "Not ours. "),
                ChatEvent.TurnStart("t9", id, null),
                ChatEvent.Text("t9", "Hello there. How "),
                ChatEvent.Text("other", "Still not ours. "),
                ChatEvent.Text("t9", "are you"),
                ChatEvent.TurnEnd("t9", "m1", false),
            )
        }.ask("hi", null).let { withTimeout(TIMEOUT_MS) { it.toList() } }
        // The voice note rides ahead of the words; HQ strips it from the saved chat.
        assertEquals(listOf(VoiceTurn.VOICE_CONTEXT + "hi"), sent)
        assertEquals(
            listOf(TurnOutput.Thread("t9"), TurnOutput.Text("Hello there. How "), TurnOutput.Text("are you"), TurnOutput.Done),
            out,
        )
    }

    @Test
    fun anotherClientsTurnStartIsIgnored() = runBlocking {
        val events = bus()
        val out = turn(events, mutableListOf()) { id ->
            listOf(
                ChatEvent.TurnStart("x", "someone-else", null),
                ChatEvent.TurnEnd("x", "m0", false),
                ChatEvent.TurnStart("t1", id, null),
                ChatEvent.TurnEnd("t1", "m1", false),
            )
        }.ask("hi", null).let { withTimeout(TIMEOUT_MS) { it.toList() } }
        assertEquals(listOf(TurnOutput.Thread("t1"), TurnOutput.Done), out)
    }

    @Test
    fun aBusyThreadIsReported() = runBlocking {
        val events = bus()
        val out = turn(events, mutableListOf()) { id -> listOf(ChatEvent.Rejected(id, "t1", "busy", running = true)) }
            .ask("hi", "t1").let { withTimeout(TIMEOUT_MS) { it.toList() } }
        assertEquals(listOf(TurnOutput.Thread("t1"), TurnOutput.Failed(VoiceTurn.BUSY)), out)
    }

    @Test
    fun noSocketFailsAtOnce() = runBlocking {
        val out = turn(bus(), mutableListOf(), accept = false) { emptyList() }.ask("hi", null).let { withTimeout(TIMEOUT_MS) { it.toList() } }
        assertEquals(listOf(TurnOutput.Failed(VoiceTurn.HQ_STOPPED)), out)
    }

    @Test
    fun aServerErrorEndsTheTurn() = runBlocking {
        val events = bus()
        val out = turn(events, mutableListOf()) { id -> listOf(ChatEvent.TurnStart("t2", id, null), ChatEvent.Failed("t2", "model down")) }
            .ask("hi", null).let { withTimeout(TIMEOUT_MS) { it.toList() } }
        assertEquals(listOf(TurnOutput.Thread("t2"), TurnOutput.Failed("model down")), out)
    }

    private companion object {
        const val TIMEOUT_MS = 5_000L
        const val DROP_AFTER_MS = 100L
    }
}
