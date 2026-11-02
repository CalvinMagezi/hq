package example.hq.spike.voice

import org.junit.Assert.assertEquals
import org.junit.Test

class SpeechWordsTest {
    @Test
    fun acronymsAreSpelled() {
        assertEquals("Hey h q, the a p i is up", SpeechWords.forVoice("Hey HQ, the APIs is up"))
    }

    @Test
    fun numbersAreWrittenOut() {
        assertEquals("You have twenty one tasks", SpeechWords.forVoice("You have 21 tasks"))
        assertEquals("one thousand two hundred thirty four notes", SpeechWords.forVoice("1,234 notes"))
        assertEquals("three point five percent", SpeechWords.forVoice("3.5%"))
        assertEquals("two million five", SpeechWords.forVoice("2000005"))
        assertEquals("one hundred", SpeechWords.forVoice("100"))
    }

    @Test
    fun longDigitRunsAreReadOneByOne() {
        assertEquals(
            "call zero seven seven one two three four five six seven eight nine zero one two zero one",
            SpeechWords.forVoice("call 07712345678901201"),
        )
    }

    @Test
    fun onlyALowercasePluralSIsDropped() {
        assertEquals("p a s s it on", SpeechWords.forVoice("PASS it on"))
    }

    @Test
    fun symbolsBecomeWords() {
        assertEquals("tea and toast at nine", SpeechWords.forVoice("tea & toast @ 9"))
    }

    @Test
    fun wordsCoverEveryScale() {
        assertEquals("zero", SpeechWords.words(0))
        assertEquals("ninety nine", SpeechWords.words(99))
        assertEquals("one billion one", SpeechWords.words(1_000_000_001))
    }
}
