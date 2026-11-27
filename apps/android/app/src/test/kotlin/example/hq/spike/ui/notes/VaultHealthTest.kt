package example.hq.spike.ui.notes

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
class VaultHealthTest {
    private val now = java.time.Instant.parse("2026-11-27T12:00:00Z").toEpochMilli()

    private fun answer(counts: String = """{"links":{"dead":0,"ambiguous":0,"broken_anchors":0},"duplicates":0,"frontmatter":0}""", lists: String = "", fixes: String = "[]", a: Int = 0, repairs: String = "[]") =
        JSONObject("""{"checked":true,"report":{"counts":$counts${if (lists.isEmpty()) "" else ",$lists"}},"fixes":$fixes,"fix_counts":{"a":$a,"b":0},"repairs":$repairs}""")

    @Test fun nothingCheckedYetOrNoSuchCheckIsNothing() {
        assertNull(VaultHealthParse.from(null, now))
        assertNull(VaultHealthParse.from(JSONObject("""{"checked":false,"report":null}"""), now))
    }

    @Test fun aHealthyCheckHasNothingToSay() {
        val h = VaultHealthParse.from(answer(), now)!!
        assertEquals(0, h.problems)
        assertFalse(h.worthShowing)
    }

    @Test fun problemsAreTheThingsAPersonCouldFixAndNotLooseNotes() {
        val counts = """{"links":{"dead":3,"ambiguous":1,"broken_anchors":2},"duplicates":1,"frontmatter":4,"loose":9,"empty":2}"""
        val h = VaultHealthParse.from(answer(counts), now)!!
        assertEquals(listOf(3, 1, 2, 1, 4), listOf(h.dead, h.ambiguous, h.anchors, h.duplicates, h.frontmatter))
        assertEquals(11, h.problems)
        assertTrue(h.worthShowing)
    }

    @Test fun aLinkFindsItsRepairByNoteAndHowItIsWritten() {
        val lists = """"dead_links":[{"note":"Notebooks/A.md","line":2,"link":"[[Old/Plan]]","target":"Old/Plan","did_you_mean":null},{"note":"Notebooks/A.md","line":3,"link":"[[Nowhere]]","target":"Nowhere","did_you_mean":"Nowhere Fast"}]"""
        val fixes = """[{"id":"f1","kind":"stale_path","tier":"a","note":"Notebooks/A.md","before":"[[Old/Plan]]"}]"""
        val h = VaultHealthParse.from(answer("""{"links":{"dead":2,"ambiguous":0,"broken_anchors":0},"duplicates":0,"frontmatter":0}""", lists, fixes, a = 1), now)!!
        assertEquals(1, h.certain)
        assertEquals("f1", h.items[0].fixId)
        assertTrue(h.items[0].sure)
        assertNull(h.items[1].fixId)
        assertEquals("Nowhere Fast", h.items[1].detail)
    }

    @Test fun aJsonNullIsNoHintAndNotTheWordNull() {
        val lists = """"dead_links":[{"note":"Notebooks/A.md","line":2,"link":"[[Gone]]","target":"Gone","did_you_mean":null}]"""
        val h = VaultHealthParse.from(answer("""{"links":{"dead":1,"ambiguous":0,"broken_anchors":0},"duplicates":0,"frontmatter":0}""", lists), now)!!
        assertNull(h.items[0].detail)
    }

    @Test fun aLikelyRepairIsNotSure() {
        val lists = """"dead_links":[{"note":"Notebooks/A.md","line":2,"link":"[[Quarterly Reveiw]]","target":"x","did_you_mean":null}]"""
        val fixes = """[{"id":"g","kind":"near_miss","tier":"b","note":"Notebooks/A.md","before":"[[Quarterly Reveiw]]"}]"""
        val h = VaultHealthParse.from(answer("""{"links":{"dead":1,"ambiguous":0,"broken_anchors":0},"duplicates":0,"frontmatter":0}""", lists, fixes), now)!!
        assertEquals("g", h.items[0].fixId)
        assertFalse(h.items[0].sure)
    }

    @Test fun aRepeatedNoteFindsItsRepairByAnyNoteInTheSet() {
        val lists = """"duplicates":[{"notes":["Notebooks/Keep.md","Notebooks/Dup.md"]}]"""
        val fixes = """[{"id":"m","kind":"merge_duplicate","tier":"b","note":"Notebooks/Dup.md","before":"Notebooks/Dup.md"}]"""
        val h = VaultHealthParse.from(answer("""{"links":{"dead":0,"ambiguous":0,"broken_anchors":0},"duplicates":1,"frontmatter":0}""", lists, fixes), now)!!
        assertEquals("Keep, Dup", h.items[0].text)
        assertEquals("m", h.items[0].fixId)
    }

    @Test fun aRecentRepairCanBeUndoneUntilItIsTakenBackOrAgesOut() {
        fun repairs(at: String, undone: String = "null", fixes: Int = 3) = """[{"run_id":"r1","ran_at":"$at","undone_at":$undone,"fixes":$fixes}]"""
        val recent = VaultHealthParse.from(answer(repairs = repairs("2026-11-27T11:00:00Z")), now)!!
        assertEquals("r1", recent.undoRun)
        assertEquals(3, recent.undoFixes)
        assertTrue("a fix that left nothing wrong still shows, to be undone", recent.worthShowing)
        assertNull(VaultHealthParse.from(answer(repairs = repairs("2026-11-27T11:00:00Z", undone = "\"2026-11-27T11:30:00Z\"")), now)!!.undoRun)
        assertNull(VaultHealthParse.from(answer(repairs = repairs("2026-11-24T11:00:00Z")), now)!!.undoRun)
        assertNull(VaultHealthParse.from(answer(repairs = repairs("2026-11-27T11:00:00Z", fixes = 0)), now)!!.undoRun)
        assertNull(VaultHealthParse.from(answer(repairs = repairs("not a time")), now)!!.undoRun)
    }
}
