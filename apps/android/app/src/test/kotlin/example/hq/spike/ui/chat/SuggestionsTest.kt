package example.hq.spike.ui.chat

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class SuggestionsTest {
    private fun cap(id: String, state: String = "ready", example: String = "Try $id") =
        """{"id":"$id","group":"notes","says":"Do $id","example":"$example","state":"$state","used":false}"""

    @Test
    fun readySuggestionsWithAnExampleAreKeptInOrder() {
        val body = """{"needs_model":false,"suggestions":[${cap("a")},${cap("b", state = "one_step")},${cap("c", example = " ")},${cap("d")}]}"""
        val chips = Suggestions.parse(body)
        assertEquals(listOf("Do a", "Do d"), chips.map { it.label })
        assertEquals("Try a", chips.first().text)
        assertEquals("notes", chips.first().group)
    }

    @Test
    fun noChipsBeforeAModelKeyAndAtMostThree() {
        assertTrue(Suggestions.parse("""{"needs_model":true,"suggestions":[${cap("a")}]}""").isEmpty())
        val many = (1..6).joinToString(",") { cap("x$it") }
        assertEquals(Suggestions.MAX, Suggestions.parse("""{"needs_model":false,"suggestions":[$many]}""").size)
    }

    @Test
    fun theFixedThreeStandInUntilHqAnswers() {
        assertEquals(Suggestions.Fallback, Suggestions.shown(null))
        assertEquals(Suggestions.Fallback, Suggestions.shown(emptyList()))
        val own = listOf(Suggestion("tasks", "Do a", "Try a"))
        assertEquals(own, Suggestions.shown(own))
    }

    @Test
    fun aPillKeepsItsCapabilityIdForFewerAndNever() {
        val chips = Suggestions.parse("""{"needs_model":false,"suggestions":[${cap("find-notes")}]}""")
        assertEquals("find-notes", chips.single().id)
        assertEquals(listOf("Do x"), Suggestions.without(listOf(Suggestion("n", "Do a", "a", "a"), Suggestion("n", "Do x", "x", "")), "a")?.map { it.label })
        assertEquals(null, Suggestions.without(null, "a"))
    }

    @Test
    fun aLongLabelIsCutAtAWordToFortyCharactersOrFewer() {
        val long = "Find every note I wrote about the quarterly planning meeting last spring"
        val cut = Suggestions.cap(long)
        assertTrue(cut.length <= Suggestions.LABEL_MAX)
        assertEquals("Find every note I wrote about the…", cut)
        assertTrue(long.startsWith(cut.dropLast(1)))
    }

    @Test
    fun aShortLabelAndOneOfExactlyFortyAreLeftAlone() {
        val forty = "a".repeat(Suggestions.LABEL_MAX)
        assertEquals(forty, Suggestions.cap(forty))
        assertEquals("What needs me today?", Suggestions.cap("What needs me today?"))
    }

    @Test
    fun oneLongWordIsCutHardWithAnEllipsisStillWithinTheCap() {
        val cut = Suggestions.cap("x".repeat(60))
        assertEquals(Suggestions.LABEL_MAX, cut.length)
        assertTrue(cut.endsWith("…"))
    }

    @Test
    fun parsedLabelsAreCapped() {
        val body = """{"needs_model":false,"suggestions":[{"id":"a","group":"notes","says":"${"word ".repeat(20).trim()}","example":"x","state":"ready"}]}"""
        assertTrue(Suggestions.parse(body).single().label.length <= Suggestions.LABEL_MAX)
    }
}
