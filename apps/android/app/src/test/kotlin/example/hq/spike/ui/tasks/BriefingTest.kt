package example.hq.spike.ui.tasks

import java.time.Instant
import org.junit.Assert.assertEquals
import org.junit.Test

class BriefingTest {
    private val now = Instant.parse("2026-10-22T12:00:00Z")

    private fun briefing(counts: List<Pair<String, Int>> = emptyList(), shown: Int = 0, reviews: List<Pair<String, Int>> = emptyList(), gaps: Int = 0, fresh: List<Pair<String, String?>> = emptyList()) =
        Briefing("Alpha", shown, counts, emptyList(), emptyList(), emptyList(), reviews, gaps, emptyList(), fresh)

    @Test fun theStatusLineCountsEachStatusAndSaysHowManyTasks() {
        assertEquals("2 in progress, 1 to do (3 tasks)", BriefingWords.statusLine(briefing(listOf("in_progress" to 2, "to_do" to 1), 3)))
        assertEquals("1 to do (1 task)", BriefingWords.statusLine(briefing(listOf("to_do" to 1), 1)))
        assertEquals("", BriefingWords.statusLine(briefing()))
    }

    @Test fun reviewStatesComeInAFixedOrderAndZeroesAreLeftOut() {
        assertEquals("2 accepted, 1 unreviewed", BriefingWords.reviewSummary(listOf("unreviewed" to 1, "accepted" to 2, "disputed" to 0)))
        assertEquals("no reviews recorded", BriefingWords.reviewSummary(emptyList()))
        assertEquals("Notes: no reviews recorded. 1 gap in attribution or links.", BriefingWords.gapsLine(briefing(gaps = 1)))
        assertEquals("Notes: 1 accepted. 2 gaps in attribution or links.", BriefingWords.gapsLine(briefing(reviews = listOf("accepted" to 1), gaps = 2)))
    }

    @Test fun ageReadsServerTimesWithAndWithoutAZone() {
        assertEquals("3h ago", BriefingWords.age("2026-10-22 09:00:00", now))
        assertEquals("12m ago", BriefingWords.age("2026-10-22T11:48:00+00:00", now))
        assertEquals("2d ago", BriefingWords.age("2026-10-20T12:00:00Z", now))
        assertEquals("just now", BriefingWords.age("2026-10-22 12:00:30", now))
        assertEquals("unknown", BriefingWords.age(null, now))
        assertEquals("unknown", BriefingWords.age("last week", now))
    }

    @Test fun nobodyIsGuessedAsTheAuthorOfANote() {
        assertEquals("claude (agent)", BriefingWords.author("recorded", "claude", "agent"))
        assertEquals("claude", BriefingWords.author("recorded", "claude", "unknown"))
        assertEquals("unknown", BriefingWords.author("no_record", null, null))
        assertEquals("unknown (edited after its last recorded write)", BriefingWords.author("changed_outside_tools", "x", null))
        assertEquals("file is gone", BriefingWords.author("missing", null, null))
    }

    @Test fun aVersionWithNoRecordIsNeverCalledUnreviewed() {
        assertEquals("No record", BriefingWords.badge(null))
        assertEquals("Unreviewed", BriefingWords.badge("unreviewed"))
        assertEquals("Accepted", BriefingWords.badge("accepted"))
    }

    @Test fun theFooterNamesEachSourceAndSaysWhenNothingCameFromOne() {
        val line = BriefingWords.freshnessLine(briefing(fresh = listOf("tasks" to "2026-10-22 11:57:00", "sessions" to null)), now)
        assertEquals(true, line.startsWith("As of tasks: 3m ago · sessions: nothing yet. Generated now."))
    }

    @Test fun contributorsAndNotesReadAsOneLine() {
        assertEquals("1 note, 2 leases, last 1h ago", BriefingWords.contributorLine(Contributor("a", 1, 2, "2026-10-22T11:00:00Z"), now))
        assertEquals("3 notes", BriefingWords.contributorLine(Contributor("a", 3, 0, null), now))
        assertEquals("HQ-12, by claude, version recorded 3h ago", BriefingWords.noteLine(BriefNote("N/a.md", "HQ-12", "claude", "2026-10-22T09:00:00Z", null), now))
    }
}
