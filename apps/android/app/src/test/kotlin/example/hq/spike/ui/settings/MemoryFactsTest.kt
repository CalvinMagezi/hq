package example.hq.spike.ui.settings

import example.hq.spike.R
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MemoryFactsTest {
    private val answer = JSONObject(
        """
        {"items":[
          {"id":12,"summary":"Acme Feeds delivers on Thursdays","created_at":"2026-11-24T08:00:00+00:00","status":"active","origin":"owner","pinned":true,"used":3},
          {"id":8,"summary":"Shop closes at 6pm","created_at":"2026-10-18T08:00:00+00:00","status":"review","origin":"unknown_channel","pinned":false,"used":0}
        ],
        "counts":{"total":2,"active":1,"review":1,"out_of_date":0,"archived":0},
        "recall":"shadow"}
        """.trimIndent(),
    )

    @Test fun theListReadsEachNoteAndTheCounts() {
        val list = MemoryFacts.parse(answer)
        assertEquals(listOf(12L, 8L), list.notes.map { it.id })
        assertEquals("2026-11-24", list.notes.first().day)
        assertTrue(list.notes.first().pinned)
        assertEquals(3, list.notes.first().used)
        assertEquals(MemoryCounts(total = 2, active = 1, review = 1, outdated = 0, archived = 0), list.counts)
        assertEquals("shadow", list.recall)
    }

    @Test fun anAnswerWithNothingInItReadsAsEmptyNotAsAFailure() {
        val list = MemoryFacts.parse(JSONObject())
        assertTrue(list.notes.isEmpty())
        assertEquals(0, list.counts.total)
    }

    @Test fun aHeldBackOrSharedNoteOffersKeepAndAnOldOneOffersUseAgain() {
        val live = MemoryNote(1, "a", "2026-11-01", false, "active", "owner", 0)
        assertFalse(MemoryFacts.heldBack(live))
        assertEquals(R.string.memory_mark_outdated to "outdated", MemoryFacts.standing(live))
        assertEquals(R.string.memory_keep to "active", MemoryFacts.standing(live.copy(status = "review")))
        assertEquals(R.string.memory_keep to "active", MemoryFacts.standing(live.copy(origin = "unknown_channel")))
        assertEquals(R.string.memory_use_again to "active", MemoryFacts.standing(live.copy(status = "archived")))
        assertEquals(R.string.memory_use_again to "active", MemoryFacts.standing(live.copy(status = "superseded")))
    }
}
