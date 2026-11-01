package example.hq.spike.voice

import example.hq.spike.data.ChatEvent
import kotlinx.coroutines.flow.MutableSharedFlow
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
    private fun turn(events: MutableSharedFlow<ChatEvent>, sent: MutableList<String>, accept: Boolean = true, script: (String) -> List<ChatEvent>) =
        VoiceTurn(events) { content, _, clientId ->
            sent += content
            if (accept) script(clientId).forEach { check(events.tryEmit(it)) }
            accept
        }

    private fun bus() = MutableSharedFlow<ChatEvent>(extraBufferCapacity = 64)

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
    }
}
