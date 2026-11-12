package example.hq.spike.ui.settings

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SuggestFactsTest {
    private val body = """
        {"days":30,"types":[
          {"type":"dated_todo","says":"Add it to your tasks","shown":2,"tapped":1,"not_now":1,"later":0,"never":0},
          {"type":"pasted_list","says":"Turn it into a note","shown":5,"tapped":2,"not_now":1,"later":1,"never":1},
          {"type":"idle","says":"Nothing","shown":0,"tapped":0,"not_now":0,"later":0,"never":0},
          {"type":"","says":"No type","shown":9}
        ]}
    """.trimIndent()

    @Test fun readsEachTypeBusiestFirstAndDropsAllZeroAndUnnamedOnes() {
        val s = SuggestFacts.parse(body)!!
        assertEquals(30, s.days)
        assertEquals(listOf("pasted_list", "dated_todo"), s.rows.map { it.type })
        assertEquals(SuggestRow("pasted_list", "Turn it into a note", 5, 2, 1, 1, 1), s.rows.first())
    }

    @Test fun aNeverCountsEvenWhenNothingWasShownInThePeriod() {
        val s = SuggestFacts.parse("""{"days":30,"types":[{"type":"find-notes","says":"","shown":0,"tapped":0,"not_now":0,"later":0,"never":1}]}""")!!
        assertEquals("find-notes", s.rows.single().says)
    }

    @Test fun anAnswerThatIsNotTheSummaryIsHqHasNotSaidYet() {
        assertNull(SuggestFacts.parse(null))
        assertNull(SuggestFacts.parse(""))
        assertNull(SuggestFacts.parse("<html>404</html>"))
        assertNull(SuggestFacts.parse("""{"error":"not found"}"""))
    }

    @Test fun anEmptyListIsAnAnswer() {
        val s = SuggestFacts.parse("""{"types":[]}""")!!
        assertTrue(s.rows.isEmpty())
        assertEquals(SuggestFacts.DEFAULT_DAYS, s.days)
    }
}
