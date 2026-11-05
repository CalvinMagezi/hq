package example.hq.spike.ui.tasks

import example.hq.spike.testing.testWords
import java.time.Instant
import java.time.ZoneOffset
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class AwayWordsTest {
    private fun briefing(mode: String = "enforce", done: Int = 0, back: Int = 0, windows: String = "[]") = Away.parse(JSONObject(
        """{"mode":"$mode","completed_total":$done,"completed":[],"reworked_total":$back,"reworked":[${(1..back).joinToString { """{"task_id":"t$it","display_id":"T-$it","title":"x"}""" }}],"veto_windows":$windows,"needs_you_total":2}""",
    ))

    @Test fun zeroPartsAreLeftOut() {
        assertEquals(1, AwayWords.card(testWords(), briefing(done = 3), { null }).size)
        assertTrue(AwayWords.card(testWords(), briefing(), { null }).isEmpty())
    }

    @Test fun theCardIsHiddenWhenOnlyWatchingOrEmpty() {
        assertFalse(briefing(mode = "shadow", done = 3).hasNews)
        assertFalse(briefing().hasNews)
        assertTrue(briefing(done = 1).hasNews)
    }

    @Test fun aReopenedTaskGoesToTheHelperOnItOrToYou() {
        val a = briefing(back = 2)
        val lines = AwayWords.wentBack(testWords(), a) { if (it == "t1") "Scout" else null }
        assertEquals(2, lines.size)
        assertTrue(lines.any { it.contains("Scout") })
        assertNull(AwayWords.sendBackTo("me"))
    }

    @Test fun aFinishAfterMidnightSaysTomorrow() {
        val now = Instant.parse("2026-01-01T20:00:00Z")
        val z = ZoneOffset.UTC
        assertFalse(AwayWords.finishes(testWords(), Instant.parse("2026-01-01T22:00:00Z").epochSecond, now, z).contains("tomorrow"))
        assertTrue(AwayWords.finishes(testWords(), Instant.parse("2026-01-02T09:00:00Z").epochSecond, now, z).contains("tomorrow"))
        val later = AwayWords.finishes(testWords(), Instant.parse("2026-01-04T09:00:00Z").epochSecond, now, z)
        assertFalse(later, later.contains("tomorrow"))
        assertTrue(later, later.startsWith("Finishes "))
    }

    @Test fun theTasksPagesNeverClaimWhatTheServerDoesNot() {
        val dir = java.io.File("src/main/kotlin/example/hq/spike/ui/tasks")
        dir.listFiles { f -> f.extension == "kt" }!!.forEach {
            val text = it.readLines().filterNot { l -> l.trim().let { t -> t.startsWith("/**") || t.startsWith("*") || t.startsWith("//") } }.joinToString("\n")
            assertFalse(it.name, text.contains("unless you stop it"))
            assertFalse(it.name, Regex("\"[^\"\n]*\\bfixed\\b").containsMatchIn(text))
        }
    }
}
