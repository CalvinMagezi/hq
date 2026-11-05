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
    fun noChipsBeforeAModelKeyAndAtMostFour() {
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
}
