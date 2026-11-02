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

class SpokenTextTest {
    @Test
    fun sentencesComeOutAsSoonAsTheyEnd() {
        val words = SpokenText()
        assertEquals(emptyList<String>(), words.add("It is 21"))
        assertEquals(listOf("It is 21 degrees."), words.add(" degrees. And"))
        assertEquals(emptyList<String>(), words.add(" sunny"))
        assertEquals(listOf("And sunny"), words.finish())
    }

    @Test
    fun markdownIsReadAsWords() {
        val words = SpokenText()
        val said = words.add("## Today\n- **Call** the [bank](https://bank.example.com)\n| a | b |\n|---|---|\nSee https://x.example.com now.\n")
        assertEquals(listOf("Today", "Call the bank", "a , b", "See a link now."), said)
    }

    @Test
    fun codeBlocksAreNamedNotRead() {
        val words = SpokenText()
        val said = words.add("Run this:\n```bash\nrm -rf build. now\n```\nDone.\n")
        assertEquals(listOf("Run this:", "There is code in the reply; it is on the screen.", "Done."), said)
    }

    @Test
    fun aLongRunWithNoEndIsSpokenAtASpace() {
        val words = SpokenText(longestWait = 20)
        val said = words.add("one two three four five six seven")
        assertEquals(listOf("one two three four"), said)
        assertEquals(listOf("five six seven"), words.finish())
    }
}

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
        assertEquals(TurnOutput.Failed(VoiceTurn.CONNECTION_LOST), out.last())
    }

    @Test
    fun aSocketStillOpeningIsNotADrop() = runBlocking {
        val events = bus()
        val connected = MutableStateFlow(false)
        val out = turn(events, mutableListOf(), connected = connected) { id ->
            listOf(ChatEvent.TurnStart("t5", id, null), ChatEvent.Text("t5", "Fine. "), ChatEvent.TurnEnd("t5", "m5", false))
        }.ask("hi", null).let { withTimeout(TIMEOUT_MS) { it.toList() } }
        assertEquals(listOf(TurnOutput.Thread("t5"), TurnOutput.Say("Fine."), TurnOutput.Done), out)
    }

    @Test
    fun missedEventsEndTheTurn() = runBlocking {
        val events = bus()
        val out = turn(events, mutableListOf()) { id -> listOf(ChatEvent.TurnStart("t4", id, null), ChatEvent.Lagged(3)) }
            .ask("hi", null).let { withTimeout(TIMEOUT_MS) { it.toList() } }
        assertEquals(listOf(TurnOutput.Thread("t4"), TurnOutput.Failed(VoiceTurn.MISSED)), out)
    }

    @Test
    fun aQuestionHqNeverPicksUpTimesOut() = runBlocking {
        val out = turn(bus(), mutableListOf(), startWaitMs = 50) { emptyList() }.ask("hi", null).let { withTimeout(TIMEOUT_MS) { it.toList() } }
        assertEquals(listOf(TurnOutput.Failed(VoiceTurn.NOT_STARTED)), out)
    }

    @Test
    fun aReplyBecomesSentencesThenDone() = runBlocking {
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
        assertEquals(listOf("hi"), sent)
        assertEquals(
            listOf(TurnOutput.Thread("t9"), TurnOutput.Say("Hello there."), TurnOutput.Say("How are you"), TurnOutput.Done),
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
        assertEquals(listOf(TurnOutput.Failed(VoiceTurn.BUSY)), out)
    }

    @Test
    fun noSocketFailsAtOnce() = runBlocking {
        val out = turn(bus(), mutableListOf(), accept = false) { emptyList() }.ask("hi", null).let { withTimeout(TIMEOUT_MS) { it.toList() } }
        assertEquals(listOf(TurnOutput.Failed(VoiceTurn.NOT_CONNECTED)), out)
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
